//! Everything that talks to GitHub, through the `gh` CLI (it already holds your login, so this
//! app never sees a token). Every function here blocks; `jobs::spawn` runs them off the UI
//! thread.

use std::path::Path;
use std::process::Command;

use std::collections::HashMap;

use crate::model::{
    Deployment, Envs, Hit, Inbox, Notif, Pr, PrDetail, PrView, QueueInfo, RemoteRepo, RepoMeta, Run, RunView, SearchRepo, parse_time,
};

/// Run a program and return stdout, or stderr (trimmed) as the error.
pub fn run(program: &str, args: &[&str], cwd: Option<&Path>) -> Result<String, String> {
    let mut cmd = Command::new(program);
    cmd.args(args);
    if let Some(dir) = cwd {
        cmd.current_dir(dir);
    }
    #[cfg(windows)]
    {
        // CREATE_NO_WINDOW: no console window flashes up for each call from a GUI app.
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x0800_0000);
    }
    let out = cmd.output().map_err(|e| format!("couldn't start `{program}`: {e}"))?;
    if out.status.success() {
        Ok(String::from_utf8_lossy(&out.stdout).into_owned())
    } else {
        let err = String::from_utf8_lossy(&out.stderr).trim().to_owned();
        Err(if err.is_empty() { format!("`{program} {}` failed", args.join(" ")) } else { err })
    }
}

fn gh_json<T: serde::de::DeserializeOwned>(args: &[&str]) -> Result<T, String> {
    let out = run("gh", args, None)?;
    serde_json::from_str(&out).map_err(|e| format!("unexpected gh output: {e}"))
}

const PR_FIELDS: &str =
    "number,title,author,headRefName,baseRefName,isDraft,reviewDecision,statusCheckRollup,updatedAt,additions,deletions,changedFiles,mergeable,state,url,labels,files";

/// `state`: open, merged, closed. `search`: GitHub search syntax over the repo's PRs
/// (title, body, author, branch...), so it finds PRs beyond the first `limit`.
pub fn prs(slug: &str, state: &str, limit: usize, search: Option<&str>) -> Result<Vec<Pr>, String> {
    let limit = limit.to_string();
    let mut args = vec!["pr", "list", "-R", slug, "--state", state, "--limit", &limit, "--json", PR_FIELDS];
    if let Some(q) = search {
        args.extend(["--search", q]);
    }
    gh_json(&args)
}

pub fn pr_detail(slug: &str, number: u64) -> Result<PrDetail, String> {
    let n = number.to_string();
    let view: PrView =
        gh_json(&["pr", "view", &n, "-R", slug, "--json", "body,state,mergeStateStatus,reviews,comments,files,statusCheckRollup"])?;
    // A diff can be huge; the view only needs the start of it.
    let mut diff = run("gh", &["pr", "diff", &n, "-R", slug], None).unwrap_or_default();
    cap(&mut diff, 400_000);
    Ok(PrDetail { view, diff })
}

pub fn runs(slug: &str, limit: usize) -> Result<Vec<Run>, String> {
    gh_json(&[
        "run", "list", "-R", slug, "--limit", &limit.to_string(), "--json",
        "databaseId,displayTitle,workflowName,headBranch,headSha,event,status,conclusion,createdAt,startedAt,updatedAt,url,number",
    ])
}

pub fn run_detail(slug: &str, id: u64) -> Result<RunView, String> {
    gh_json(&["run", "view", &id.to_string(), "-R", slug, "--json", "jobs"])
}

pub fn envs(slug: &str) -> Result<Envs, String> {
    let (owner, name) = slug.split_once('/').ok_or("expected owner/name")?;
    let q = "query($o:String!,$n:String!){repository(owner:$o,name:$n){environments(first:50){nodes{name}} \
             deployments(first:100,orderBy:{field:CREATED_AT,direction:DESC}){nodes{environment commitOid createdAt \
             ref{name} creator{login} latestStatus{state}}}}}";
    let v: serde_json::Value =
        gh_json(&["api", "graphql", "-f", &format!("query={q}"), "-F", &format!("o={owner}"), "-F", &format!("n={name}")])?;
    let repo = &v["data"]["repository"];
    let s = |v: &serde_json::Value| v.as_str().unwrap_or_default().to_owned();
    let names = repo["environments"]["nodes"].as_array().into_iter().flatten().map(|n| s(&n["name"])).collect();
    let deployments = repo["deployments"]["nodes"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|d| Deployment {
            environment: s(&d["environment"]),
            sha: s(&d["commitOid"]),
            git_ref: s(&d["ref"]["name"]),
            creator: s(&d["creator"]["login"]),
            created_at: parse_time(d["createdAt"].as_str().unwrap_or_default()).unwrap_or(0),
            state: d["latestStatus"]["state"].as_str().unwrap_or("PENDING").to_owned(),
        })
        .collect();
    // REST has the protection rules; GraphQL above doesn't. Missing access just means "unknown".
    let protected = run("gh", &["api", &format!("repos/{slug}/environments"), "--jq",
        ".environments[] | select([.protection_rules[]?.type] | any(. == \"required_reviewers\" or . == \"wait_timer\")) | .name"], None)
        .map(|o| o.lines().map(str::to_owned).collect())
        .unwrap_or_default();
    Ok(Envs { names, protected, deployments })
}

/// Your login and every repo you can reach: yours and each organization's, listed in
/// parallel (one `gh repo list` per owner), so it takes about as long as the slowest one.
pub fn all_repos() -> Result<(String, Vec<RemoteRepo>), String> {
    let me: String = run("gh", &["api", "user", "--jq", ".login"], None)?.trim().to_owned();
    let orgs = run("gh", &["api", "user/orgs", "--paginate", "--jq", ".[].login"], None).unwrap_or_default();
    let owners: Vec<String> = std::iter::once(me.clone()).chain(orgs.lines().map(str::trim).filter(|l| !l.is_empty()).map(str::to_owned)).collect();
    let jobs: Vec<_> = owners
        .into_iter()
        .map(|owner| {
            std::thread::spawn(move || -> Result<Vec<RemoteRepo>, String> {
                gh_json(&["repo", "list", &owner, "--limit", "2000", "--json", "nameWithOwner,pushedAt,isArchived"])
            })
        })
        .collect();
    let mut all = Vec::new();
    let mut first_err = None;
    for j in jobs {
        match j.join().map_err(|_| "repo list thread panicked".to_string()).and_then(|r| r) {
            Ok(mut r) => all.append(&mut r),
            // One org refusing (SSO, OAuth restrictions) shouldn't hide the others.
            Err(e) => first_err = first_err.or(Some(e)),
        }
    }
    if all.is_empty()
        && let Some(e) = first_err
    {
        return Err(e);
    }
    Ok((me, all))
}

/// Any repository on GitHub you can see, for the sidebar's search.
pub fn search_repos(q: &str) -> Result<Vec<SearchRepo>, String> {
    gh_json(&["search", "repos", q, "--limit", "20", "--json", "fullName,pushedAt,description"])
}

const HIT_FIELDS: &str = "repository,number,title,author,updatedAt,url,isDraft";

/// Your day: PRs waiting on you, your open PRs, assigned issues, and recent runs of your most
/// recently pushed repos. The four searches run in parallel.
pub fn inbox(active_repos: Vec<String>) -> Result<Inbox, String> {
    let search = |kind: &'static str, filter: &'static str| {
        std::thread::spawn(move || -> Result<Vec<Hit>, String> {
            let fields = if kind == "issues" { "repository,number,title,author,updatedAt,url" } else { HIT_FIELDS };
            gh_json(&["search", kind, filter, "--state=open", "--limit", "30", "--sort", "updated", "--json", fields])
        })
    };
    let review = search("prs", "--review-requested=@me");
    let mine = search("prs", "--author=@me");
    let issues = search("issues", "--assignee=@me");
    let runs: Vec<_> = active_repos
        .into_iter()
        .map(|slug| std::thread::spawn(move || runs_head(&slug, 4).map(|r| r.into_iter().map(|x| (slug.clone(), x)).collect::<Vec<_>>())))
        .collect();
    let join = |h: std::thread::JoinHandle<Result<Vec<Hit>, String>>| h.join().map_err(|_| "search thread panicked".to_string())?;
    let mut inbox = Inbox { review_requested: join(review)?, my_prs: join(mine)?, issues: join(issues)?, runs: Vec::new() };
    for h in runs {
        // A repo without Actions (or without access) just contributes nothing.
        if let Ok(Ok(mut r)) = h.join() {
            inbox.runs.append(&mut r);
        }
    }
    inbox.runs.sort_by(|a, b| b.1.created_at.cmp(&a.1.created_at));
    Ok(inbox)
}

fn runs_head(slug: &str, n: usize) -> Result<Vec<Run>, String> {
    gh_json(&[
        "run", "list", "-R", slug, "--limit", &n.to_string(), "--json",
        "databaseId,displayTitle,workflowName,headBranch,event,status,conclusion,createdAt,startedAt,updatedAt,url,number",
    ])
}

// ---------------------------------------------------------------------------------------------
// Actions on GitHub (write)
// ---------------------------------------------------------------------------------------------

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MergeMethod {
    Squash,
    Merge,
    Rebase,
}

impl MergeMethod {
    pub const ALL: [MergeMethod; 3] = [MergeMethod::Squash, MergeMethod::Merge, MergeMethod::Rebase];
    pub fn label(self) -> &'static str {
        match self {
            MergeMethod::Squash => "Squash",
            MergeMethod::Merge => "Merge commit",
            MergeMethod::Rebase => "Rebase",
        }
    }
    fn flag(self) -> &'static str {
        match self {
            MergeMethod::Squash => "--squash",
            MergeMethod::Merge => "--merge",
            MergeMethod::Rebase => "--rebase",
        }
    }
}

pub fn approve(slug: &str, n: u64, body: &str) -> Result<String, String> {
    let n = n.to_string();
    let mut args = vec!["pr", "review", &n, "-R", slug, "--approve"];
    if !body.is_empty() {
        args.extend(["-b", body]);
    }
    run("gh", &args, None)
}

pub fn request_changes(slug: &str, n: u64, body: &str) -> Result<String, String> {
    run("gh", &["pr", "review", &n.to_string(), "-R", slug, "--request-changes", "-b", body], None)
}

pub fn comment(slug: &str, n: u64, body: &str) -> Result<String, String> {
    run("gh", &["pr", "comment", &n.to_string(), "-R", slug, "-b", body], None)
}

pub fn merge(slug: &str, n: u64, method: MergeMethod, delete_branch: bool) -> Result<String, String> {
    let n = n.to_string();
    let mut args = vec!["pr", "merge", &n, "-R", slug, method.flag()];
    if delete_branch {
        args.push("--delete-branch");
    }
    run("gh", &args, None)
}

pub fn close(slug: &str, n: u64) -> Result<String, String> {
    run("gh", &["pr", "close", &n.to_string(), "-R", slug], None)
}

/// `gh pr checkout` in the local clone: fetches the PR's branch and switches to it.
pub fn checkout_pr(dir: &Path, slug: &str, n: u64) -> Result<String, String> {
    run("gh", &["pr", "checkout", &n.to_string(), "-R", slug], Some(dir))
}

pub fn rerun(slug: &str, id: u64, failed_only: bool) -> Result<String, String> {
    let id = id.to_string();
    let mut args = vec!["run", "rerun", &id, "-R", slug];
    if failed_only {
        args.push("--failed");
    }
    run("gh", &args, None)
}

pub fn cancel(slug: &str, id: u64) -> Result<String, String> {
    run("gh", &["run", "cancel", &id.to_string(), "-R", slug], None)
}

/// Cut `s` to at most `max` bytes at a line end.
pub fn cap(s: &mut String, max: usize) {
    if s.len() > max {
        let mut max = max;
        while !s.is_char_boundary(max) {
            max -= 1;
        }
        let cut = s[..max].rfind('\n').unwrap_or(max);
        s.truncate(cut);
        s.push_str("\n… (cut here: too large to preview)");
    }
}

// ---------------------------------------------------------------------------------------------
// Guard rails (A2): default branch, protected branches, admin, rulesets
// ---------------------------------------------------------------------------------------------

pub fn repo_meta(slug: &str) -> Result<RepoMeta, String> {
    let head = run("gh", &["api", &format!("repos/{slug}"), "--jq", ".default_branch + \"\\t\" + ((.permissions.admin // false) | tostring)"], None)?;
    let (default_branch, admin) = head.trim().split_once('\t').unwrap_or((head.trim(), "false"));
    // Readable with read access (unlike the full protection settings, which need admin).
    let protected = run("gh", &["api", &format!("repos/{slug}/branches?protected=true&per_page=100"), "--paginate", "--jq", ".[].name"], None)
        .map(|o| o.lines().map(str::to_owned).collect())
        .unwrap_or_default();
    Ok(RepoMeta { default_branch: default_branch.to_owned(), protected, admin: admin == "true" })
}

/// The rule types that apply to `branch` ("deletion", "non_fast_forward", "pull_request", ...).
pub fn branch_rules(slug: &str, branch: &str) -> Vec<String> {
    run("gh", &["api", &format!("repos/{slug}/rules/branches/{branch}"), "--jq", ".[].type"], None)
        .map(|o| {
            let mut v: Vec<String> = o.lines().map(str::to_owned).collect();
            v.dedup();
            v
        })
        .unwrap_or_default()
}

pub fn reopen(slug: &str, n: u64) -> Result<String, String> {
    run("gh", &["pr", "reopen", &n.to_string(), "-R", slug], None)
}

// ---------------------------------------------------------------------------------------------
// B1: notifications
// ---------------------------------------------------------------------------------------------

pub fn notifications() -> Result<Vec<Notif>, String> {
    let v: serde_json::Value = gh_json(&["api", "notifications?per_page=50"])?;
    let s = |v: &serde_json::Value| v.as_str().unwrap_or_default().to_owned();
    Ok(v.as_array()
        .into_iter()
        .flatten()
        .map(|n| {
            let url = s(&n["subject"]["url"]);
            Notif {
                id: s(&n["id"]),
                reason: s(&n["reason"]),
                unread: n["unread"].as_bool().unwrap_or(true),
                updated_at: s(&n["updated_at"]),
                title: s(&n["subject"]["title"]),
                kind: s(&n["subject"]["type"]),
                repo: s(&n["repository"]["full_name"]),
                number: url.rsplit('/').next().and_then(|x| x.parse().ok()),
            }
        })
        .collect())
}

/// Mark a notification thread read on GitHub ("Done").
pub fn mark_read(id: &str) -> Result<String, String> {
    run("gh", &["api", "-X", "PATCH", &format!("notifications/threads/{id}")], None).map(|_| "Marked as done".into())
}

// ---------------------------------------------------------------------------------------------
// B2: review queue facts, one GraphQL query for every PR
// ---------------------------------------------------------------------------------------------

pub fn queue_info(prs: &[(String, u64)]) -> Result<HashMap<(String, u64), QueueInfo>, String> {
    if prs.is_empty() {
        return Ok(HashMap::new());
    }
    let mut q = String::from("query{");
    for (i, (slug, n)) in prs.iter().enumerate() {
        let Some((o, r)) = slug.split_once('/') else { continue };
        q.push_str(&format!(
            "p{i}:repository(owner:{o:?},name:{r:?}){{pullRequest(number:{n}){{createdAt additions deletions \
             timelineItems(itemTypes:[REVIEW_REQUESTED_EVENT],last:1){{nodes{{...on ReviewRequestedEvent{{createdAt}}}}}} \
             reviewRequests(first:10){{nodes{{requestedReviewer{{...on User{{login}} ...on Team{{name}}}}}}}} \
             commits(last:1){{nodes{{commit{{statusCheckRollup{{state}}}}}}}}}}}}"
        ));
    }
    q.push('}');
    let v: serde_json::Value = gh_json(&["api", "graphql", "-f", &format!("query={q}")])?;
    let mut out = HashMap::new();
    for (i, key) in prs.iter().enumerate() {
        let pr = &v["data"][format!("p{i}")]["pullRequest"];
        if pr.is_null() {
            continue;
        }
        let t = |x: &serde_json::Value| x.as_str().and_then(parse_time);
        let requested = t(&pr["timelineItems"]["nodes"][0]["createdAt"]).or_else(|| t(&pr["createdAt"])).unwrap_or(0);
        let waiting_on = pr["reviewRequests"]["nodes"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|n| n["requestedReviewer"]["login"].as_str().or(n["requestedReviewer"]["name"].as_str()).map(str::to_owned))
            .collect();
        out.insert(
            key.clone(),
            QueueInfo {
                requested_at: requested,
                additions: pr["additions"].as_u64().unwrap_or(0),
                deletions: pr["deletions"].as_u64().unwrap_or(0),
                ci: pr["commits"]["nodes"][0]["commit"]["statusCheckRollup"]["state"].as_str().unwrap_or("").to_owned(),
                waiting_on,
            },
        );
    }
    Ok(out)
}

// ---------------------------------------------------------------------------------------------
// B4: CI triage
// ---------------------------------------------------------------------------------------------

/// The failed steps' log, last `lines` lines, without the job/step/timestamp prefix columns.
pub fn failed_log(slug: &str, id: u64, lines: usize) -> Result<String, String> {
    let mut out = run("gh", &["run", "view", &id.to_string(), "-R", slug, "--log-failed"], None)?;
    cap(&mut out, 2_000_000);
    let cleaned: Vec<String> = out
        .lines()
        .map(|l| {
            // "job\tstep\t2026-11-14T08:36:07.1234567Z message"
            let mut parts = l.splitn(3, '\t');
            let (job, step, rest) = (parts.next().unwrap_or(""), parts.next().unwrap_or(""), parts.next().unwrap_or(l));
            let msg = rest.split_once(' ').filter(|(ts, _)| ts.len() > 20 && ts.ends_with('Z')).map_or(rest, |(_, m)| m);
            // gh labels steps it can't match "UNKNOWN STEP": say only the job then.
            if step.is_empty() || step == "UNKNOWN STEP" { format!("[{job}] {msg}") } else { format!("[{job} › {step}] {msg}") }
        })
        .collect();
    // Centre on the last error line, not the tail: after a failure come the cleanup steps.
    let is_error = |l: &str| {
        let l = l.to_lowercase();
        ["##[error]", "error:", "error[", "failed", "panicked", "fatal:", "exception"].iter().any(|w| l.contains(w))
    };
    let end = cleaned.iter().rposition(|l| is_error(l)).map_or(cleaned.len(), |i| (i + 6).min(cleaned.len()));
    let start = end.saturating_sub(lines);
    Ok(cleaned[start..end].join("\n"))
}

// ---------------------------------------------------------------------------------------------
// B5 / B6: group dashboards and watching
// ---------------------------------------------------------------------------------------------

/// Runs on one branch (a group dashboard's default-branch health, an auto-watched branch).
pub fn runs_on(slug: &str, branch: &str, limit: usize) -> Result<Vec<Run>, String> {
    gh_json(&[
        "run", "list", "-R", slug, "--branch", branch, "--limit", &limit.to_string(), "--json",
        "databaseId,displayTitle,workflowName,headBranch,headSha,event,status,conclusion,createdAt,startedAt,updatedAt,url,number",
    ])
}

/// One run's status (for watching).
pub fn run_one(slug: &str, id: u64) -> Result<Run, String> {
    gh_json(&[
        "run", "view", &id.to_string(), "-R", slug, "--json",
        "databaseId,displayTitle,workflowName,headBranch,headSha,event,status,conclusion,createdAt,startedAt,updatedAt,url,number",
    ])
}

/// One PR's state (for watching).
pub fn pr_one(slug: &str, n: u64) -> Result<Pr, String> {
    gh_json(&["pr", "view", &n.to_string(), "-R", slug, "--json", "number,title,state,reviewDecision,statusCheckRollup,url,updatedAt"])
}

/// How many commits `head` has that `base` doesn't (deploy drift: default branch vs deployed sha).
pub fn ahead_by(slug: &str, base: &str, head: &str) -> Option<u32> {
    run("gh", &["api", &format!("repos/{slug}/compare/{base}...{head}"), "--jq", ".ahead_by"], None).ok()?.trim().parse().ok()
}

// ---------------------------------------------------------------------------------------------
// Settings → Accounts
// ---------------------------------------------------------------------------------------------

/// A GitHub account the gh CLI is signed in to.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Account {
    pub host: String,
    pub login: String,
    /// gh uses this one for its host.
    pub active: bool,
    /// The token works.
    pub ok: bool,
    pub scopes: String,
    /// keyring, oauth_token, GH_TOKEN…
    pub source: String,
    /// https or ssh.
    pub protocol: String,
}

/// Every account `gh auth status` knows, active ones first.
pub fn accounts() -> Result<Vec<Account>, String> {
    let out = run("gh", &["auth", "status", "--json", "hosts"], None)?;
    Ok(parse_accounts(&out))
}

fn parse_accounts(json: &str) -> Vec<Account> {
    let v: serde_json::Value = serde_json::from_str(json).unwrap_or_default();
    let mut out = Vec::new();
    for (host, list) in v["hosts"].as_object().into_iter().flatten() {
        for a in list.as_array().into_iter().flatten() {
            let s = |k: &str| a[k].as_str().unwrap_or_default().to_owned();
            out.push(Account {
                host: host.clone(),
                login: s("login"),
                active: a["active"].as_bool().unwrap_or(false),
                ok: s("state") == "success",
                scopes: s("scopes"),
                source: s("tokenSource"),
                protocol: s("gitProtocol"),
            });
        }
    }
    out.sort_by_key(|a| (!a.active, a.host.clone(), a.login.clone()));
    out
}

/// Make `login` the account gh uses on `host`.
pub fn switch_account(host: &str, login: &str) -> Result<String, String> {
    run("gh", &["auth", "switch", "-h", host, "-u", login], None).map(|_| format!("Now using {login} on {host}"))
}

/// First lines of `gh --version` and `git --version`.
pub fn versions() -> (String, String) {
    let first = |p: &str| run(p, &["--version"], None).ok().and_then(|o| o.lines().next().map(str::to_owned)).unwrap_or_else(|| format!("{p}: not found"));
    (first("gh"), first("git"))
}

#[cfg(test)]
mod account_tests {
    #[test]
    fn parses_gh_auth_status() {
        let j = r#"{"hosts":{"github.com":[{"state":"success","active":false,"host":"github.com","login":"b","tokenSource":"keyring","scopes":"repo","gitProtocol":"https"},{"state":"success","active":true,"host":"github.com","login":"a","tokenSource":"keyring","scopes":"repo, workflow","gitProtocol":"ssh"}]}}"#;
        let v = super::parse_accounts(j);
        assert_eq!((v[0].login.as_str(), v[0].active, v[0].protocol.as_str()), ("a", true, "ssh"));
        assert_eq!(v[1].login, "b");
        assert!(super::parse_accounts("not json").is_empty());
    }
}
