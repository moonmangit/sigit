//! Everything that talks to GitHub, through the `gh` CLI (it already holds your login, so this
//! app never sees a token). Every function here blocks; `jobs::spawn` runs them off the UI
//! thread.

use std::path::Path;
use std::process::Command;

use std::collections::HashMap;

use crate::model::{
    Deployment, Envs, Hit, Inbox, Notif, Pr, PrDetail, PrView, QueueInfo, RemoteRepo, RepoMeta, Run, RunView, SearchRepo, parse_time,
};

// ---------------------------------------------------------------------------------------------
// The commands the app runs, as you'd type them (Settings → Safety → Show gh / git commands)
// ---------------------------------------------------------------------------------------------

/// A command an action runs: its program and arguments. Building the arguments in one place
/// keeps what the app shows ([`Cmd::line`]) the same as what it runs ([`Cmd::run`]).
#[derive(Clone, Debug, PartialEq)]
pub struct Cmd {
    pub program: &'static str,
    pub args: Vec<String>,
}

impl Cmd {
    pub fn new(program: &'static str, args: &[&str]) -> Cmd {
        Cmd { program, args: args.iter().map(|a| (*a).to_owned()).collect() }
    }

    pub fn arg(mut self, a: impl Into<String>) -> Cmd {
        self.args.push(a.into());
        self
    }

    pub fn run(&self, cwd: Option<&Path>) -> Result<String, String> {
        run(self.program, &self.args.iter().map(String::as_str).collect::<Vec<_>>(), cwd)
    }

    /// As you'd type it in a terminal: arguments with spaces or quotes are quoted, and a long
    /// one (a review comment, say) is cut short.
    pub fn line(&self) -> String {
        line(self.program, &self.args.iter().map(String::as_str).collect::<Vec<_>>())
    }
}

/// `program args…` as a shell line.
pub fn line(program: &str, args: &[&str]) -> String {
    let quote = |a: &str| {
        let a = if a.chars().count() > 60 { format!("{}…", a.chars().take(57).collect::<String>()) } else { a.to_owned() };
        let a = a.replace('\n', "↵");
        if a.is_empty() || a.contains([' ', '"', '\'', '$', '`', '|', '&', ';', '<', '>', '(', ')', '*', '?', '#', '~']) {
            format!("\"{}\"", a.replace('\\', "\\\\").replace('"', "\\\""))
        } else {
            a
        }
    };
    std::iter::once(program.to_owned()).chain(args.iter().map(|a| quote(a))).collect::<Vec<_>>().join(" ")
}

thread_local! {
    /// While [`recording`] runs on this thread: every command started, as a line.
    static RECORD: std::cell::RefCell<Option<Vec<String>>> = const { std::cell::RefCell::new(None) };
}

/// Runs `f` and returns what it returned with every gh / git command it started, in order.
pub fn recording<R>(f: impl FnOnce() -> R) -> (R, Vec<String>) {
    RECORD.with(|r| *r.borrow_mut() = Some(Vec::new()));
    let out = f();
    let lines = RECORD.with(|r| r.borrow_mut().take()).unwrap_or_default();
    (out, lines)
}

/// Run a program and return stdout, or stderr (trimmed) as the error.
/// macOS hands an app opened from Finder or the Dock only `/usr/bin:/bin:/usr/sbin:/sbin`, so a
/// Homebrew `gh` (in `/opt/homebrew/bin` or `/usr/local/bin`) can't be found. Take the PATH your
/// login shell builds, plus those folders, and use it for every command the app runs. Call it
/// first thing in `main`, before any thread starts.
#[cfg(target_os = "macos")]
pub fn fix_path() {
    use std::time::{Duration, Instant};
    let mut dirs: Vec<String> = Vec::new();
    let shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/zsh".to_owned());
    // `-l` reads the profile that sets up Homebrew; give up after 3s in case a profile hangs.
    if let Ok(mut child) = Command::new(&shell)
        .args(["-l", "-c", "printf %s \"$PATH\""])
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .spawn()
    {
        let start = Instant::now();
        loop {
            match child.try_wait() {
                Ok(Some(_)) => {
                    let mut out = String::new();
                    if let Some(mut so) = child.stdout.take() {
                        let _ = std::io::Read::read_to_string(&mut so, &mut out);
                    }
                    dirs.extend(out.trim().split(':').filter(|d| !d.is_empty()).map(str::to_owned));
                    break;
                }
                Ok(None) if start.elapsed() < Duration::from_secs(3) => std::thread::sleep(Duration::from_millis(20)),
                _ => {
                    let _ = child.kill();
                    let _ = child.wait();
                    break;
                }
            }
        }
    }
    dirs.extend(["/opt/homebrew/bin", "/usr/local/bin"].map(str::to_owned));
    dirs.extend(std::env::var("PATH").unwrap_or_default().split(':').filter(|d| !d.is_empty()).map(str::to_owned));
    let mut seen = std::collections::HashSet::new();
    dirs.retain(|d| seen.insert(d.clone()));
    // SAFETY: called at the top of `main`, before any other thread exists.
    unsafe { std::env::set_var("PATH", dirs.join(":")) };
}

/// A GitHub profile picture's bytes, fetched with `gh api` (no other HTTP client in the app).
/// Only GitHub's avatar host is allowed, so nothing else is ever fetched this way.
pub fn avatar_bytes(url: &str) -> Result<Vec<u8>, String> {
    if !url.starts_with(AVATAR_HOST) {
        return Err(format!("not a GitHub avatar: {url}"));
    }
    let out = command("gh", &["api", url], None).output().map_err(|e| format!("couldn't start `gh`: {e}"))?;
    if out.status.success() && !out.stdout.is_empty() { Ok(out.stdout) } else { Err(String::from_utf8_lossy(&out.stderr).trim().to_owned()) }
}

pub const AVATAR_HOST: &str = "https://avatars.githubusercontent.com/";

/// `program args` in `cwd`, without a console window flashing up on Windows.
pub(crate) fn command(program: &str, args: &[&str], cwd: Option<&Path>) -> Command {
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
        // The PATH as it is now, not as it was when sigit started: Windows only hands an updated
        // PATH to programs started after an installer changes it, so a `gh` installed while the
        // setup screen is open would never be found. Rust looks the program up in the child's
        // PATH when it's set here.
        cmd.env("PATH", fresh_path());
    }
    cmd
}

/// The PATH sigit started with, then the system and user PATH as Windows has them now (where
/// installers write), then the folders gh and git install into by default; each folder once.
#[cfg(windows)]
fn fresh_path() -> std::ffi::OsString {
    let env = |k: &str| std::env::var(k).unwrap_or_default();
    let defaults = [
        format!(r"{}\GitHub CLI", env("ProgramFiles")),
        format!(r"{}\Git\cmd", env("ProgramFiles")),
        format!(r"{}\Microsoft\WinGet\Links", env("LOCALAPPDATA")),
        format!(r"{}\scoop\shims", env("USERPROFILE")),
    ];
    let current = std::env::var_os("PATH").map(|p| p.to_string_lossy().into_owned()).unwrap_or_default();
    let machine = registry_path(true).unwrap_or_default();
    let user = registry_path(false).unwrap_or_default();
    merge_path(&[&current, &machine, &user, &defaults.join(";")]).into()
}

/// Folders from each `;`-separated list in order, each once (Windows paths: case and a
/// trailing `\` don't matter); empty entries dropped.
pub(crate) fn merge_path(lists: &[&str]) -> String {
    let mut seen = std::collections::HashSet::new();
    let mut out: Vec<&str> = Vec::new();
    for dir in lists.iter().flat_map(|l| l.split(';')).map(str::trim).filter(|d| !d.is_empty()) {
        if seen.insert(dir.trim_end_matches('\\').to_lowercase()) {
            out.push(dir);
        }
    }
    out.join(";")
}

/// `Path` from the registry, with `%VARIABLES%` expanded: the machine one (`HKLM\...\Session
/// Manager\Environment`) or the user one (`HKCU\Environment`).
#[cfg(windows)]
fn registry_path(machine: bool) -> Option<String> {
    use windows_sys::Win32::System::Environment::ExpandEnvironmentStringsW;
    use windows_sys::Win32::System::Registry::{HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE, RRF_NOEXPAND, RRF_RT_REG_EXPAND_SZ, RRF_RT_REG_SZ, RegGetValueW};
    let wide = |s: &str| s.encode_utf16().chain([0]).collect::<Vec<u16>>();
    let (root, key) = if machine {
        (HKEY_LOCAL_MACHINE, wide(r"SYSTEM\CurrentControlSet\Control\Session Manager\Environment"))
    } else {
        (HKEY_CURRENT_USER, wide("Environment"))
    };
    let name = wide("Path");
    let flags = RRF_RT_REG_SZ | RRF_RT_REG_EXPAND_SZ | RRF_NOEXPAND;
    let mut bytes: u32 = 0;
    // SAFETY: plain registry reads into a buffer of the size Windows reports.
    unsafe {
        if RegGetValueW(root, key.as_ptr(), name.as_ptr(), flags, std::ptr::null_mut(), std::ptr::null_mut(), &mut bytes) != 0 || bytes == 0 {
            return None;
        }
        let mut raw = vec![0u16; bytes as usize / 2 + 1];
        if RegGetValueW(root, key.as_ptr(), name.as_ptr(), flags, std::ptr::null_mut(), raw.as_mut_ptr().cast(), &mut bytes) != 0 {
            return None;
        }
        let len = ExpandEnvironmentStringsW(raw.as_ptr(), std::ptr::null_mut(), 0);
        if len == 0 {
            return None;
        }
        let mut out = vec![0u16; len as usize];
        ExpandEnvironmentStringsW(raw.as_ptr(), out.as_mut_ptr(), len);
        let end = out.iter().position(|&c| c == 0).unwrap_or(out.len());
        Some(String::from_utf16_lossy(&out[..end]))
    }
}

pub fn run(program: &str, args: &[&str], cwd: Option<&Path>) -> Result<String, String> {
    RECORD.with(|r| {
        if let Some(v) = r.borrow_mut().as_mut() {
            v.push(line(program, args));
        }
    });
    let out = command(program, args, cwd).output().map_err(|e| format!("couldn't start `{program}`: {e}"))?;
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

pub fn approve_cmd(slug: &str, n: u64, body: &str) -> Cmd {
    let c = Cmd::new("gh", &["pr", "review", &n.to_string(), "-R", slug, "--approve"]);
    if body.is_empty() { c } else { c.arg("-b").arg(body) }
}

pub fn request_changes_cmd(slug: &str, n: u64, body: &str) -> Cmd {
    Cmd::new("gh", &["pr", "review", &n.to_string(), "-R", slug, "--request-changes", "-b", body])
}

pub fn comment_cmd(slug: &str, n: u64, body: &str) -> Cmd {
    Cmd::new("gh", &["pr", "comment", &n.to_string(), "-R", slug, "-b", body])
}

pub fn merge_cmd(slug: &str, n: u64, method: MergeMethod, delete_branch: bool) -> Cmd {
    let c = Cmd::new("gh", &["pr", "merge", &n.to_string(), "-R", slug, method.flag()]);
    if delete_branch { c.arg("--delete-branch") } else { c }
}

pub fn close_cmd(slug: &str, n: u64) -> Cmd {
    Cmd::new("gh", &["pr", "close", &n.to_string(), "-R", slug])
}

/// `gh pr checkout`, run in the local clone: fetches the PR's branch and switches to it.
pub fn checkout_pr_cmd(slug: &str, n: u64) -> Cmd {
    Cmd::new("gh", &["pr", "checkout", &n.to_string(), "-R", slug])
}

pub fn rerun_cmd(slug: &str, id: u64, failed_only: bool) -> Cmd {
    let c = Cmd::new("gh", &["run", "rerun", &id.to_string(), "-R", slug]);
    if failed_only { c.arg("--failed") } else { c }
}

pub fn cancel_cmd(slug: &str, id: u64) -> Cmd {
    Cmd::new("gh", &["run", "cancel", &id.to_string(), "-R", slug])
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

pub fn reopen_cmd(slug: &str, n: u64) -> Cmd {
    Cmd::new("gh", &["pr", "reopen", &n.to_string(), "-R", slug])
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
pub fn mark_read_cmd(id: &str) -> Cmd {
    Cmd::new("gh", &["api", "-X", "PATCH", &format!("notifications/threads/{id}")])
}

pub fn mark_read(id: &str) -> Result<String, String> {
    mark_read_cmd(id).run(None).map(|_| "Marked as done".into())
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
    let mut out = failed_log_cmd(slug, id).run(None)?;
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
/// One PR with everything the list has (for a link to a PR outside the loaded list).
pub fn pr_full(slug: &str, n: u64) -> Result<Pr, String> {
    gh_json(&["pr", "view", &n.to_string(), "-R", slug, "--json", PR_FIELDS])
}

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
pub fn switch_account_cmd(host: &str, login: &str) -> Cmd {
    Cmd::new("gh", &["auth", "switch", "-h", host, "-u", login])
}

pub fn switch_account(host: &str, login: &str) -> Result<String, String> {
    switch_account_cmd(host, login).run(None).map(|_| format!("Now using {login} on {host}"))
}

/// The failing steps' log of a run ("Why it failed").
pub fn failed_log_cmd(slug: &str, id: u64) -> Cmd {
    Cmd::new("gh", &["run", "view", &id.to_string(), "-R", slug, "--log-failed"])
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

#[cfg(test)]
mod command_tests {
    use super::*;
    use crate::guard::Action;

    #[test]
    fn merge_path_keeps_order_and_drops_repeats() {
        let merged = merge_path(&[r"C:\a;C:\B\;", r"c:\b;C:\Program Files\GitHub CLI\", r";C:\a\;D:\x"]);
        assert_eq!(merged, r"C:\a;C:\B\;C:\Program Files\GitHub CLI\;D:\x");
        assert_eq!(merge_path(&["", ";;"]), "");
    }

    /// The registry PATH is read as Windows has it now: a folder only there (not in this
    /// process's PATH) still ends up in what child processes get.
    #[cfg(windows)]
    #[test]
    fn fresh_path_includes_the_registry_path() {
        let machine = registry_path(true).expect("the machine PATH is readable");
        let first = machine.split(';').find(|d| !d.trim().is_empty()).expect("a folder").trim().trim_end_matches('\\').to_lowercase();
        let fresh = fresh_path().to_string_lossy().to_lowercase();
        assert!(fresh.split(';').any(|d| d.trim_end_matches('\\') == first), "{first} missing from {fresh}");
    }

    /// Changes this process's PATH, so it's ignored by default. Run it with the bare PATH a
    /// Finder launch gets:
    /// `env -i HOME=$HOME PATH=/usr/bin:/bin:/usr/sbin:/sbin <test binary> --ignored finds_homebrew_gh`
    #[cfg(target_os = "macos")]
    #[test]
    #[ignore]
    fn finds_homebrew_gh() {
        fix_path();
        let out = run("gh", &["--version"], None).expect("gh should be found after fix_path");
        assert!(out.starts_with("gh version"), "{out}");
    }

    #[test]
    fn lines_read_like_a_terminal() {
        assert_eq!(merge_cmd("o/r", 12, MergeMethod::Squash, true).line(), "gh pr merge 12 -R o/r --squash --delete-branch");
        assert_eq!(comment_cmd("o/r", 3, "looks good, ship it").line(), "gh pr comment 3 -R o/r -b \"looks good, ship it\"");
        assert_eq!(approve_cmd("o/r", 3, "").line(), "gh pr review 3 -R o/r --approve");
        // A long body is cut, a newline shown as ↵, quotes escaped.
        let long = comment_cmd("o/r", 1, &"x".repeat(80)).line();
        assert!(long.ends_with("x…") && long.chars().count() < 100, "{long}");
        assert_eq!(comment_cmd("o/r", 1, "a \"b\"\nc").line(), "gh pr comment 1 -R o/r -b \"a \\\"b\\\"↵c\"");
    }

    /// What an action shows is what it runs: the shown lines come from the same `Cmd`s.
    #[test]
    fn actions_show_what_they_run() {
        let dir = std::path::PathBuf::from(".");
        let co = Action::Checkout { dir: dir.clone(), slug: "o/r".into(), name: "origin/feat".into(), remote: true };
        assert_eq!(co.command_lines(false), ["git switch feat", "git switch --track origin/feat"]);
        let del = Action::DeleteBranch { dir: dir.clone(), name: "old".into(), sha: "abc".into(), force: true, unpushed: Vec::new() };
        assert_eq!(del.command_lines(false), ["git branch -D old"]);
        let pull = Action::Pull { dir };
        let stashed = pull.command_lines(true);
        assert_eq!(stashed[0], "git stash push --include-untracked -m \"sigit: before Pull\"");
        assert_eq!(stashed[1], "git pull --ff-only");
    }

    /// Every action and every undo names the gh / git command it runs (the `match`es in
    /// `commands` / `command_lines` are exhaustive, so a new action can't skip it).
    #[test]
    fn every_action_and_undo_shows_a_command() {
        use crate::guard::Undo;
        let (dir, s) = (std::path::PathBuf::from("."), || "o/r".to_owned());
        let actions = [
            Action::Merge { slug: s(), number: 1, title: String::new(), method: MergeMethod::Rebase, delete_branch: false },
            Action::Close { slug: s(), number: 1, title: String::new() },
            Action::Approve { slug: s(), number: 1, title: String::new(), body: String::new(), confirm: true },
            Action::RequestChanges { slug: s(), number: 1, body: "fix".into() },
            Action::Comment { slug: s(), number: 1, body: "hi".into() },
            Action::Rerun { slug: s(), id: 9, title: String::new(), workflow: String::new(), sha: String::new(), failed_only: true },
            Action::CancelRun { slug: s(), id: 9, title: String::new(), workflow: String::new(), sha: String::new() },
            Action::Checkout { dir: dir.clone(), slug: s(), name: "dev".into(), remote: false },
            Action::PrCheckout { dir: dir.clone(), slug: s(), number: 1 },
            Action::Pull { dir: dir.clone() },
            Action::Fetch { dir: dir.clone() },
            Action::Stash { dir: dir.clone(), op: "drop", name: "stash@{1}".into() },
            Action::DeleteBranch { dir: dir.clone(), name: "x".into(), sha: "abc".into(), force: false, unpushed: Vec::new() },
        ];
        let expected = [
            "gh pr merge 1 -R o/r --rebase",
            "gh pr close 1 -R o/r",
            "gh pr review 1 -R o/r --approve",
            "gh pr review 1 -R o/r --request-changes -b fix",
            "gh pr comment 1 -R o/r -b hi",
            "gh run rerun 9 -R o/r --failed",
            "gh run cancel 9 -R o/r",
            "git switch dev",
            "gh pr checkout 1 -R o/r",
            "git pull --ff-only",
            "git fetch --all --prune",
            "git stash drop stash@{1}",
            "git branch -d x",
        ];
        for (a, want) in actions.iter().zip(expected) {
            assert_eq!(a.command_lines(false), [want], "{}", a.label());
        }
        let undos = [
            (Undo::ReopenPr { slug: s(), number: 1 }, "gh pr reopen 1 -R o/r"),
            (Undo::SwitchBack { dir: dir.clone(), branch: "main".into() }, "git switch main"),
            (Undo::RestoreBranch { dir: dir.clone(), name: "x".into(), sha: "abc".into() }, "git branch x abc"),
            (Undo::StashPop { dir }, "git stash pop stash@{0}"),
        ];
        for (u, want) in undos {
            assert_eq!(u.command_lines(), [want]);
        }
        // The rest of the buttons that run something.
        assert_eq!(mark_read_cmd("42").line(), "gh api -X PATCH notifications/threads/42");
        assert_eq!(switch_account_cmd("github.com", "me").line(), "gh auth switch -h github.com -u me");
        assert_eq!(failed_log_cmd("o/r", 9).line(), "gh run view 9 -R o/r --log-failed");
        assert_eq!(crate::git::set_global_cmd("user.name", "Ann Lee").line(), "git config --global user.name \"Ann Lee\"");
        assert!(crate::update::download_cmd("v1", "a.zip", "d").line().starts_with("gh release download v1 --repo "));
    }

    #[test]
    fn records_what_ran() {
        let (_, ran) = recording(|| {
            let _ = run("git", &["--version"], None);
            let _ = run("git", &["config", "--get", "user.name"], None);
        });
        assert_eq!(ran, ["git --version", "git config --get user.name"]);
        // Nothing is recorded outside `recording`.
        let (_, none) = recording(|| ());
        assert!(none.is_empty());
    }
}
