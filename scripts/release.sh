#!/usr/bin/env sh
# Build a release binary to share (macOS / Linux):  ./scripts/release.sh
#
# Rust embeds the full path of every source file it compiles (for panic messages), which would
# put your user name and folders into the binary ("/Users/<you>/.cargo/..."). This maps those
# paths to neutral names first. Output: target/release/sigit
set -eu
project=$(cd "$(dirname "$0")/.." && pwd)
cargo_home=${CARGO_HOME:-$HOME/.cargo}
rustup_home=${RUSTUP_HOME:-$HOME/.rustup}
# When several prefixes match, rustc uses the last: general first, specific last.
export RUSTFLAGS="--remap-path-prefix=$HOME=home --remap-path-prefix=$cargo_home=cargo --remap-path-prefix=$rustup_home=rustup --remap-path-prefix=$project=src"
cd "$project"
cargo build --release
# Check: no trace of this machine's home folder may be left in the binary.
if grep -q -a -F "$HOME" target/release/sigit; then
    echo "The binary still contains your home folder path; don't share it." >&2
    exit 1
fi
echo "Built $project/target/release/sigit (no local paths inside)"
