//! Both CDP transports resolve their TLS roots from the host, never from a bundle.
//!
//! gladius speaks TLS on two transports: `reqwest` for `http://` / `https://`
//! endpoints (the attach probe and `fetch.rs`) and `tokio-tungstenite` for the
//! `ws://` / `wss://` CDP socket. They are independent rustls users, and rustls
//! takes no root source on its own - each half is told which one when features
//! are selected.
//!
//! `reqwest` 0.13 ships no bundled-roots rustls feature (CHANGELOG, 0.7.0), so
//! that selection had to move. Leaving `rustls-tls-webpki-roots` on the ws half
//! would have put `wss://` on the bundled Mozilla set while `https://` used the
//! platform verifier: one endpoint accepted on one transport and refused on the
//! other, for the same host.
//!
//! Feature selection and the lock are what decide this, so that is what the test
//! pins. It is a declaration test on purpose: a handshake against a live
//! certificate needs a CA this repo does not own, and the invariant is settled
//! entirely before any connection is made.
//!
//! Scope: it asserts the mechanism - native roots on both halves, no bundled
//! root crate in the resolved graph - and not that the two halves hold one
//! shared object. `rustls-native-certs` and `rustls-platform-verifier` are two
//! readers of the host store; `Cargo.toml` says exactly that and stops there.

use std::path::PathBuf;

fn repo_file(name: &str) -> String {
    let mut p = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    p.push(name);
    std::fs::read_to_string(&p).unwrap_or_else(|e| panic!("read {}: {e}", p.display()))
}

/// The declaration line for one dependency, comments excluded.
fn dep_line(manifest: &str, name: &str) -> String {
    let needle = format!("{name} = ");
    manifest
        .lines()
        .find(|l| l.starts_with(&needle))
        .unwrap_or_else(|| panic!("Cargo.toml declares `{name}`"))
        .to_string()
}

#[test]
fn the_ws_transport_takes_its_roots_from_the_host_store() {
    let line = dep_line(&repo_file("Cargo.toml"), "tokio-tungstenite");
    assert!(
        line.contains("rustls-tls-native-roots"),
        "the `wss://` half must select rustls-tls-native-roots: {line}"
    );
    assert!(
        !line.contains("rustls-tls-webpki-roots"),
        "the `wss://` half must not select the bundled root set: {line}"
    );
}

#[test]
fn the_http_transport_takes_its_roots_from_the_host_store() {
    let line = dep_line(&repo_file("Cargo.toml"), "reqwest");
    assert!(
        line.contains("\"rustls\""),
        "`reqwest` must select the `rustls` transport, which is the platform \
         verifier: {line}"
    );
    assert!(
        !line.contains("rustls-no-provider"),
        "`reqwest` must keep its crypto provider, so not `rustls-no-provider`: {line}"
    );
}

#[test]
fn no_bundled_root_crate_is_in_the_resolved_graph() {
    let lock = repo_file("Cargo.lock");
    assert!(
        !lock.contains("name = \"webpki-roots\""),
        "the bundled Mozilla root crate must stay out of the graph: its presence \
         means one transport trusts certificates the other does not"
    );
    for crate_name in ["rustls-native-certs", "rustls-platform-verifier"] {
        assert!(
            lock.contains(&format!("name = \"{crate_name}\"")),
            "`{crate_name}` must be in the lock: it is how a transport reads the \
             host store"
        );
    }
}
