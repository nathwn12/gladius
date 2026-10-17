mod browser;
mod fetch;
mod mcp;
mod selftest;
mod tools;

#[tokio::main]
async fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
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
    let code = match arg {
        None | Some("mcp") => match mcp::serve().await {
            Ok(()) => 0,
            Err(e) => {
                eprintln!("[gladius] {e}");
                1
            }
        },
        Some("selftest") => selftest::run().await,
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
