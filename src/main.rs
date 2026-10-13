mod browser;
mod fetch;
mod mcp;
mod selftest;
mod tools;

#[tokio::main]
async fn main() {
    let arg = std::env::args().nth(1);
    let code = match arg.as_deref() {
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
