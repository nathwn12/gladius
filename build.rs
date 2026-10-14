// Build-time provenance stamp.
//
// `gladius --version` prints the crate version plus the exact commit this
// binary was built from, so a deployed `gladius.exe` can be compared against
// `git rev-parse HEAD` instead of being guessed at from its mtime. The values
// are read through `env!` at compile time, never at run time, so the stamp
// cannot drift from the artifact it is embedded in.
//
// The sha carries a `-dirty` marker when the tree held uncommitted changes at
// build time. A source tarball with no `.git` (or no `git` on PATH) stamps
// `unknown` rather than failing the build.

use std::process::Command;

fn main() {
    // Re-stamp when the checked-out commit, the staged tree, or the sources
    // change. `src` is listed so a plain source edit (the common dirty case)
    // refreshes the marker instead of leaving the previous stamp in place.
    println!("cargo:rerun-if-changed=.git/HEAD");
    println!("cargo:rerun-if-changed=.git/index");
    // `.git/HEAD` holds only the ref *name* (`ref: refs/heads/<branch>`), so a
    // commit that advances the branch moves the ref file and leaves HEAD's
    // mtime untouched. Without this watch an empty commit (or any commit that
    // changes no source path cargo already tracks) kept the previous stamp,
    // shipping a binary labelled for a commit that is no longer HEAD. Watch the
    // resolved ref - and `packed-refs` for the packed case - so any move of the
    // commit we stamp is seen.
    if let Some(reference) = git(&["symbolic-ref", "-q", "HEAD"]) {
        println!("cargo:rerun-if-changed=.git/{reference}");
    }
    println!("cargo:rerun-if-changed=.git/packed-refs");
    println!("cargo:rerun-if-changed=src");

    let sha = git(&["rev-parse", "HEAD"]).unwrap_or_else(|| "unknown".to_string());
    let dirty = git(&["status", "--porcelain"]).is_some_and(|s| !s.is_empty());

    println!("cargo:rustc-env=GLADIUS_GIT_SHA={sha}");
    println!("cargo:rustc-env=GLADIUS_GIT_DIRTY={}", u8::from(dirty));
}

/// Run `git` in the package root; `None` when git is absent or the command
/// failed, so a git-less build still succeeds with `unknown`.
fn git(args: &[&str]) -> Option<String> {
    let out = Command::new("git")
        .args(args)
        .current_dir(env!("CARGO_MANIFEST_DIR"))
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    Some(String::from_utf8_lossy(&out.stdout).trim().to_string())
}
