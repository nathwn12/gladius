#![recursion_limit = "256"]

mod browser;
mod capture;
mod fetch;
mod layout;
mod mcp;
mod selftest;
mod tools;
mod visual;

#[tokio::main]
async fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
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
    eprintln!(
        "[gladius] engine gladius {} ({}{})",
        env!("CARGO_PKG_VERSION"),
        env!("GLADIUS_GIT_SHA"),
        match env!("GLADIUS_GIT_DIRTY") {
            "1" => "-dirty",
            _ => "",
        }
    );
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
        Some("selftest") => {
            let handle = std::thread::Builder::new()
                .name("gladius-selftest".to_string())
                .stack_size(16 * 1024 * 1024)
                .spawn(|| {
                    tokio::runtime::Builder::new_current_thread()
                        .enable_all()
                        .build()
                        .expect("selftest runtime")
                        .block_on(selftest::run())
                });
            match handle {
                Ok(h) => h.join().unwrap_or(1),
                Err(e) => {
                    eprintln!("[gladius] selftest thread: {e}");
                    1
                }
            }
        }
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
