
use std::process::Command;

fn main() {
    println!("cargo:rerun-if-changed=.git/HEAD");
    println!("cargo:rerun-if-changed=.git/index");
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
