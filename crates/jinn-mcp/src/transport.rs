//! Transport helpers for connecting to MCP servers.
//!
//! Currently this module covers HTTP-mode concerns: allocating a free local
//! port for a managed child-process server (so jinn can hand it an explicit
//! port rather than trusting the server to report where it bound).

use std::net::{SocketAddr, TcpListener};

/// Allocate a free TCP port on `bind_addr` via bind-and-release.
///
/// Binds to `(bind_addr, 0)` (the OS picks a free port), reads the assigned
/// port, then drops the listener. The caller is expected to spawn the MCP
/// server immediately with this port as a CLI argument.
///
/// The release→rebind window is instant on localhost (no `TIME_WAIT` delay),
/// so the child re-binding the same port succeeds. The only failure mode is an
/// unrelated process grabbing that exact port in the microsecond between
/// release and the child's bind — astronomically rare on localhost, and a clean
/// retry if it happens.
///
/// # Errors
///
/// Returns an error if binding to `(bind_addr, 0)` fails (e.g. `bind_addr` is
/// not a valid local address).
pub fn pick_free_port(bind_addr: &str) -> Result<u16, std::io::Error> {
    let ip: std::net::IpAddr = bind_addr
        .parse()
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidInput, e))?;
    let listener = TcpListener::bind(SocketAddr::from((ip, 0)))?;
    let port = listener.local_addr()?.port();
    drop(listener);
    Ok(port)
}

/// Expand `<ip>` and `<port>` replacement tokens in command arguments.
///
/// Mirrors the lifecycle-script token convention. Each `<ip>` substring is
/// replaced with `ip`, and each `<port>` substring with the decimal `port`.
/// Arguments with no tokens pass through unchanged. Pure and allocation-only.
pub fn expand_tokens(args: &[String], ip: &str, port: u16) -> Vec<String> {
    args.iter()
        .map(|a| a.replace("<ip>", ip).replace("<port>", &port.to_string()))
        .collect()
}

/// Extracts the host portion from a URL template like `http://127.0.0.1:<port>/mcp`.
///
/// Strips the scheme prefix and takes everything up to the first `:` (port
/// separator) or `/` (path separator), whichever comes first. Returns the raw
/// host string without validation — callers pass it to [`pick_free_port`] which
/// parses it as an IP address.
#[must_use]
pub fn parse_host(url_template: &str) -> String {
    let after_scheme = url_template.split_once("://").map_or(url_template, |(_, rest)| rest);
    let host = after_scheme.split_once(':').map_or(after_scheme, |(host, _)| host);
    host.split_once('/').map_or(host, |(host, _)| host).to_owned()
}
#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used, reason = "test assertions")]
    use super::*;

    #[test]
    fn pick_free_port_returns_a_rebindable_port() {
        // When allocating a port on loopback.
        let port = pick_free_port("127.0.0.1").expect("allocate port");

        // Then the same port can be re-bound immediately (bind-and-release works).
        let rebind = TcpListener::bind(("127.0.0.1", port));
        assert!(
            rebind.is_ok(),
            "port {port} should be immediately rebindable"
        );
    }

    #[test]
    fn pick_free_port_returns_distinct_ports_across_calls() {
        // When allocating two ports in quick succession.
        let a = pick_free_port("127.0.0.1").expect("allocate port a");
        let b = pick_free_port("127.0.0.1").expect("allocate port b");

        // Then they are distinct (OS rotates assigned ports).
        assert_ne!(a, b);
    }

    #[test]
    fn pick_free_port_rejects_invalid_bind_addr() {
        // When allocating with an invalid address.
        let result = pick_free_port("not-an-ip");

        // Then it errors.
        assert!(result.is_err());
    }

    #[test]
    fn expand_tokens_replaces_ip_and_port() {
        // Given args with both tokens.
        let args = vec![
            "server.js".to_owned(),
            "--port".to_owned(),
            "<port>".to_owned(),
            "--host".to_owned(),
            "<ip>".to_owned(),
        ];

        // When expanding.
        let out = expand_tokens(&args, "127.0.0.1", 42365);

        // Then tokens are replaced with the bind addr and port.
        assert_eq!(
            out,
            vec!["server.js", "--port", "42365", "--host", "127.0.0.1"]
        );
    }

    #[test]
    fn expand_tokens_passes_through_args_without_tokens() {
        // Given args with no tokens.
        let args = vec!["--stdio".to_owned(), "--verbose".to_owned()];

        // When expanding.
        let out = expand_tokens(&args, "127.0.0.1", 42365);

        // Then args pass through unchanged.
        assert_eq!(out, vec!["--stdio", "--verbose"]);
    }

    #[test]
    fn parse_host_extracts_from_url_with_port_token() {
        // Given a url template with <port> token.
        // When parsing the host.
        let host = parse_host("http://127.0.0.1:<port>/mcp");

        // Then the host is extracted before the port.
        assert_eq!(host, "127.0.0.1");
    }

    #[test]
    fn parse_host_extracts_from_url_without_path() {
        // Given a url template with no path.
        // When parsing the host.
        let host = parse_host("http://0.0.0.0:<port>");

        // Then the host is extracted.
        assert_eq!(host, "0.0.0.0");
    }

    #[test]
    fn expand_tokens_handles_multiple_tokens_per_arg() {
        // Given an arg containing both tokens (e.g. a combined URL-ish arg).
        let args = vec!["http://<ip>:<port>/mcp".to_owned()];

        // When expanding.
        let out = expand_tokens(&args, "0.0.0.0", 8080);

        // Then both tokens in the same arg are replaced.
        assert_eq!(out, vec!["http://0.0.0.0:8080/mcp"]);
    }
}
