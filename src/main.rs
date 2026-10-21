mod browser;
mod fetch;
mod mcp;
mod selftest;
mod tools;

#[tokio::main]
async fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    // Version handshake (fail closed): the npm launcher pins the engine it
    // provisioned, but nothing in the protocol binds the two — a stale
    // launcher once ran a stale engine silently. When the launcher exports
    // GLADIUS_EXPECTED_VERSION (its own package version), a mismatch refuses
    // to serve instead of running mixed versions. Unset = local/dev run, no
    // gate. Launcher-side export is still required for this to bite.
    if let Ok(expected) = std::env::var("GLADIUS_EXPECTED_VERSION") {
        let actual = env!("CARGO_PKG_VERSION");
        if !expected.trim().is_empty() && expected.trim() != actual {
            eprintln!(
                "[gladius] version mismatch: launcher {expected}, engine {actual} \
                 — reinstall with `npx -y gladius-mcp@latest` (then re-run --doctor); refusing to serve"
            );
            std::process::exit(3);
        }
    }
    // One stderr line on every startup so a harness can record exactly which
    // engine ran. Stderr only: stdout belongs to the MCP channel in serve.
    eprintln!(
        "[gladius] engine gladius {} ({}{})",
        env!("CARGO_PKG_VERSION"),
        env!("GLADIUS_GIT_SHA"),
        match env!("GLADIUS_GIT_DIRTY") {
            "1" => "-dirty",
            _ => "",
        }
    );
    // No headed path exists (headless-only by owner decision 2026-10-17):
    // a headed request fails closed here, before anything resolves,
    // provisions, or launches.
    if args.iter().any(|a| a == "--headed" || a == "headed") {
        eprintln!(
            "[gladius] headed mode is not supported: gladius is headless-only by owner decision \
             (2026-10-17); the owned chrome-headless-shell artifact cannot go headed. \
             Attach to an externally started engine with GLADIUS_CDP_URL instead"
        );
        std::process::exit(2);
    }
    let arg = args.first().map(|s| s.as_str());
    // The serve/selftest futures host every tool chain, including the
    // browser launch + ws-connect depth. `#[tokio::main]`'s block_on polls
    // its future on the MAIN thread, whose stack is the 1 MB PE default on
    // Windows - the launch path sits right at that edge, so a layout shift
    // (any rebuild) can flip a cold start between run and stack overflow.
    // Spawn onto a runtime worker (2 MB stack) so the depth has headroom no
    // rebuild can take away. Observed 2026-10-18: cold `navigate` overflowed
    // main deterministically on one build and ran clean on the next.
    let code = match arg {
        None | Some("mcp") => match tokio::spawn(mcp::serve()).await {
            Ok(Ok(())) => 0,
            Ok(Err(e)) => {
                eprintln!("[gladius] {e}");
                1
            }
            Err(e) => {
                eprintln!("[gladius] serve task failed: {e}");
                1
            }
        },
        Some("selftest") => match tokio::spawn(selftest::run()).await {
            Ok(c) => c,
            Err(e) => {
                eprintln!("[gladius] selftest task failed: {e}");
                1
            }
        },
        // Provenance: the crate version plus the commit this binary was built
        // from (`build.rs`), with `-dirty` when the tree was modified at build
        // time. This is the referent a deployed binary can be checked against.
        Some("--version") | Some("version") => {
            let stamp = match env!("GLADIUS_GIT_DIRTY") {
                "1" => "-dirty",
                _ => "",
            };
            println!(
                "gladius {} ({}{stamp})",
                env!("CARGO_PKG_VERSION"),
                env!("GLADIUS_GIT_SHA")
            );
            0
        }
        Some(other) => {
            eprintln!("usage: gladius [mcp|selftest|--version]; unknown subcommand: {other}");
            2
        }
    };
    std::process::exit(code);
}
