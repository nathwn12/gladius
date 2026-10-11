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
        Some(other) => {
            eprintln!("usage: gladius [mcp|selftest]; unknown subcommand: {other}");
            2
        }
    };
    std::process::exit(code);
}
