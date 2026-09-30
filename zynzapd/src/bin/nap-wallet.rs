//! `nap-wallet` — Nap Wallet as a local process with a page on loopback.
//!
//! Serves `zynzapd::app` over loopback HTTP: the page at `/`, the API under
//! `/api/`. Keys never leave the process. A browser extension may call the
//! API too (its origin is `chrome-extension://…`), which is why the server
//! answers CORS preflights for extension origins and no others.
//!
//! ```text
//!   ZYN_APP_DIR     ~/.zyn/app          keys and state
//!   ZYN_APP_WALLET  $ZYN_APP_DIR/wallet the Zcash recovery record (+ .state)
//!   ZYN_APP_KEY     $ZYN_APP_DIR/zyn.key the Zyn account key
//!   ZYN_LIGHTD      168.119.53.39:8098   compact-block server
//!   ZYN_NODE        168.119.53.39:8099   Zyn node RPC
//!   ZYN_CHAIN       11
//!   ZYN_VAULT       utest1h6sf…x2t407    the vault's deposit address
//!   ZYN_APP_LISTEN  127.0.0.1:8977
//!   ZYN_AGENT_LISTEN 127.0.0.1:8978  restricted agent API only
//!   ZYN_AGENT_TOKEN random secret shared with `nap-agent`
//! ```

use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::PathBuf;
use std::sync::Arc;

use serde_json::{json, Value};
use zynzapd::app::{api, App, Config};
use zynzapd::client::{account, hex};
use zynzapd::wallet::network_name;

const UI: &str = include_str!("../../ui/index.html");
const NAP_MARK: &[u8] = include_bytes!("../../ui/nap-mark.png");
const BRICOLAGE_GROTESQUE: &[u8] = include_bytes!("../../ui/fonts/BricolageGrotesque.ttf");
const IBM_PLEX_SANS: &[u8] = include_bytes!("../../ui/fonts/IBMPlexSans.ttf");
const IBM_PLEX_MONO_REGULAR: &[u8] = include_bytes!("../../ui/fonts/IBMPlexMono-Regular.ttf");
const IBM_PLEX_MONO_MEDIUM: &[u8] = include_bytes!("../../ui/fonts/IBMPlexMono-Medium.ttf");

fn env(k: &str, d: &str) -> String {
    std::env::var(k).unwrap_or_else(|_| d.to_string())
}

fn main() {
    let dir = PathBuf::from(env(
        "ZYN_APP_DIR",
        &format!("{}/.zyn/app", env("HOME", ".")),
    ));
    let cfg = Config::in_dir(dir);
    let listen = env("ZYN_APP_LISTEN", "127.0.0.1:8977");
    let agent_listen = env("ZYN_AGENT_LISTEN", "127.0.0.1:8978");
    let agent_token = env("ZYN_AGENT_TOKEN", "");
    if agent_token.len() < 32 {
        eprintln!("nap-wallet: ZYN_AGENT_TOKEN must contain at least 32 characters");
        std::process::exit(1);
    }
    let app = Arc::new(App::open(&cfg).unwrap_or_else(|e| {
        eprintln!("nap-wallet: {}", e);
        std::process::exit(1)
    }));
    {
        let w = app.wallet.lock().unwrap();
        eprintln!(
            "nap-wallet: {} wallet {}  account {}  node {} chain {}",
            network_name(w.network()),
            w.address(),
            hex(&account(&app.key())),
            cfg.node,
            cfg.chain
        );
    }
    let agent_listener = TcpListener::bind(&agent_listen).expect("bind agent API");
    let agent_app = Arc::clone(&app);
    let agent_secret = Arc::new(agent_token);
    let agent_secret_for_thread = Arc::clone(&agent_secret);
    std::thread::spawn(move || {
        for stream in agent_listener.incoming().flatten() {
            let app = Arc::clone(&agent_app);
            let secret = Arc::clone(&agent_secret_for_thread);
            std::thread::spawn(move || {
                let _ = handle(&app, stream, true, &secret);
            });
        }
    });
    let listener = TcpListener::bind(&listen).expect("bind wallet UI");
    eprintln!("nap-wallet: open http://{}/", listen);
    eprintln!("nap-wallet: restricted agent API on {}", agent_listen);
    if allowed_origins().is_empty() {
        eprintln!(
            "nap-wallet: ZYN_EXTENSION_ORIGINS is unset — any installed browser \
             extension may call this wallet. Set it to the wallet extension's \
             origin before this is somebody's real money."
        );
    }
    for stream in listener.incoming().flatten() {
        let app = Arc::clone(&app);
        let secret = Arc::clone(&agent_secret);
        std::thread::spawn(move || {
            let _ = handle(&app, stream, false, &secret);
        });
    }
}

struct Request {
    method: String,
    path: String,
    body: Vec<u8>,
    same_origin: bool,
    origin: Option<String>,
    agent_token: Option<String>,
}

fn read_request(s: &mut TcpStream) -> Option<Request> {
    s.set_read_timeout(Some(std::time::Duration::from_secs(10)))
        .ok();
    let mut buf = Vec::new();
    let mut tmp = [0u8; 4096];
    let head_end = loop {
        let n = s.read(&mut tmp).ok()?;
        if n == 0 {
            return None;
        }
        buf.extend_from_slice(&tmp[..n]);
        if let Some(p) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
            break p + 4;
        }
        if buf.len() > 64 << 10 {
            return None;
        }
    };
    let head = String::from_utf8_lossy(&buf[..head_end]).to_string();
    let mut lines = head.lines();
    let mut req = lines.next()?.split_whitespace();
    let method = req.next()?.to_string();
    let path = req.next()?.split('?').next()?.to_string();
    let mut len = 0usize;
    let mut same_origin = false;
    let mut origin = None;
    let mut agent_token = None;
    for l in lines {
        let Some((k, v)) = l.split_once(':') else {
            continue;
        };
        match k.trim().to_ascii_lowercase().as_str() {
            "content-length" => len = v.trim().parse().unwrap_or(0),
            "x-zyn" => same_origin = true,
            "origin" => origin = Some(v.trim().to_string()),
            "x-nap-agent-token" => agent_token = Some(v.trim().to_string()),
            _ => {}
        }
    }
    if len > 1 << 20 {
        return None;
    }
    let mut body = buf[head_end..].to_vec();
    while body.len() < len {
        let n = s.read(&mut tmp).ok()?;
        if n == 0 {
            break;
        }
        body.extend_from_slice(&tmp[..n]);
    }
    body.truncate(len);
    Some(Request {
        method,
        path,
        body,
        same_origin,
        origin,
        agent_token,
    })
}

/// Compare a secret without letting the clock describe it.
///
/// Over loopback a timing signal is a poor channel, but `!=` on a token is
/// the kind of thing that is free to get right and awkward to explain later.
fn secret_eq(a: &str, b: &str) -> bool {
    let (a, b) = (a.as_bytes(), b.as_bytes());
    if a.len() != b.len() {
        return false;
    }
    a.iter().zip(b).fold(0u8, |d, (x, y)| d | (x ^ y)) == 0
}

/// Which extension origins may call this wallet.
///
/// `ZYN_EXTENSION_ORIGINS` is a comma-separated allowlist of exact origins.
/// Set it in any real deployment: without it **every** installed extension is
/// answered, so a hostile one that asks for `127.0.0.1` host permission can
/// drive the wallet. That is a high-privilege attacker already, which is why
/// this is defence in depth rather than the only lock — but "our extension"
/// is what the comment above this function always claimed, and an allowlist
/// is what makes it true.
fn allowed_origins() -> Vec<String> {
    std::env::var("ZYN_EXTENSION_ORIGINS")
        .ok()
        .map(|v| {
            v.split(',')
                .map(|o| o.trim().trim_end_matches('/').to_string())
                .filter(|o| !o.is_empty())
                .collect()
        })
        .unwrap_or_default()
}

fn is_extension_origin(o: &str) -> bool {
    o.starts_with("chrome-extension://")
        || o.starts_with("moz-extension://")
        || o.starts_with("safari-web-extension://")
}

/// An extension is the only cross-origin caller we answer.
fn cors_with(origin: &Option<String>, allowed: &[String]) -> String {
    match origin {
        Some(o)
            if is_extension_origin(o)
                && (allowed.is_empty() || allowed.iter().any(|a| a == o.trim_end_matches('/'))) =>
        {
            format!("Access-Control-Allow-Origin: {}\r\nAccess-Control-Allow-Headers: Content-Type, X-Zyn\r\nAccess-Control-Allow-Methods: GET, POST, OPTIONS\r\nVary: Origin\r\n", o)
        }
        _ => String::new(),
    }
}

fn cors(origin: &Option<String>) -> String {
    cors_with(origin, &allowed_origins())
}

fn respond(
    s: &mut TcpStream,
    status: &str,
    ctype: &str,
    extra: &str,
    body: &[u8],
) -> std::io::Result<()> {
    write!(s, "HTTP/1.1 {}\r\nContent-Type: {}\r\nContent-Length: {}\r\nCache-Control: no-store\r\n{}Connection: close\r\n\r\n", status, ctype, body.len(), extra)?;
    s.write_all(body)?;
    s.flush()
}

/// Routes that hand back recovery material, or destroy it.
///
/// `/api/export` returns the mnemonic. An extension that can reach it can
/// exfiltrate the wallet outright, which is a different order of thing from
/// driving it — so these answer the wallet's own page only, whatever the
/// origin allowlist says. The page has no `Origin` on a same-origin POST;
/// an extension always sends one.
fn same_origin_only(path: &str) -> bool {
    matches!(
        path,
        "/api/export" | "/api/export-file" | "/api/reset" | "/api/zyn-key" | "/api/zyn-key/migrate"
    )
}

fn agent_route_allowed(method: &str, path: &str) -> bool {
    matches!(
        (method, path),
        ("GET", "/api/agent/status")
            | ("GET", "/api/agent/portfolio")
            | ("GET", "/api/agent/assets")
            | ("GET", "/api/agent/pools")
            | ("GET", "/api/agent/offers")
            | ("GET", "/api/agent/anchors")
            | ("GET", "/api/agent/mandates")
            | ("POST", "/api/agent/quote")
            | ("POST", "/api/agent/draft-swap")
            | ("POST", "/api/agent/check-swap")
            | ("POST", "/api/agent/execute-swap")
            | ("POST", "/api/agent/action")
            | ("POST", "/api/agent/pause")
            | ("POST", "/api/agent/close")
    )
}

fn static_asset(path: &str) -> Option<(&'static str, &'static [u8])> {
    match path {
        "/nap-mark.png" => Some(("image/png", NAP_MARK)),
        "/fonts/BricolageGrotesque.ttf" => Some(("font/ttf", BRICOLAGE_GROTESQUE)),
        "/fonts/IBMPlexSans.ttf" => Some(("font/ttf", IBM_PLEX_SANS)),
        "/fonts/IBMPlexMono-Regular.ttf" => Some(("font/ttf", IBM_PLEX_MONO_REGULAR)),
        "/fonts/IBMPlexMono-Medium.ttf" => Some(("font/ttf", IBM_PLEX_MONO_MEDIUM)),
        _ => None,
    }
}

fn handle(
    app: &Arc<App>,
    mut s: TcpStream,
    agent_only: bool,
    agent_secret: &str,
) -> std::io::Result<()> {
    let Some(req) = read_request(&mut s) else {
        return Ok(());
    };
    if agent_only {
        if !agent_route_allowed(&req.method, &req.path) {
            return respond(
                &mut s,
                "404 Not Found",
                "text/plain",
                "",
                b"not available on the agent service",
            );
        }
        if !req.agent_token.as_deref().is_some_and(|t| secret_eq(t, agent_secret)) {
            return respond(
                &mut s,
                "403 Forbidden",
                "text/plain",
                "",
                b"invalid agent token",
            );
        }
    }
    let cors = cors(&req.origin);
    if req.method == "OPTIONS" {
        return respond(&mut s, "204 No Content", "text/plain", &cors, b"");
    }
    if req.method == "GET" && req.path == "/" {
        return respond(
            &mut s,
            "200 OK",
            "text/html; charset=utf-8",
            "",
            UI.as_bytes(),
        );
    }
    if req.method == "GET" {
        if let Some((content_type, body)) = static_asset(&req.path) {
            return respond(&mut s, "200 OK", content_type, "", body);
        }
    }
    if !req.path.starts_with("/api/") {
        return respond(&mut s, "404 Not Found", "text/plain", "", b"not here");
    }
    // Recovery material never crosses an origin, allowlist or not.
    if !agent_only && same_origin_only(&req.path) && req.origin.is_some() {
        return respond(
            &mut s,
            "403 Forbidden",
            "text/plain",
            "",
            b"this route answers the wallet's own page only",
        );
    }
    if !agent_only && req.method == "POST" && !req.same_origin {
        // Only the page (or an extension, after a preflight) can set a
        // custom header; a cross-site form cannot.
        return respond(
            &mut s,
            "403 Forbidden",
            "text/plain",
            &cors,
            b"missing X-Zyn header",
        );
    }
    let input: Value = if req.body.is_empty() {
        json!({})
    } else {
        serde_json::from_slice(&req.body).unwrap_or(json!({}))
    };
    let (status, body) = match api(app, &req.method, &req.path, &input) {
        Ok(v) => ("200 OK", v),
        Err(e) => {
            app.note(format!("error: {}", e));
            ("400 Bad Request", json!({ "error": e }))
        }
    };
    respond(
        &mut s,
        status,
        "application/json",
        &cors,
        body.to_string().as_bytes(),
    )
}

#[cfg(test)]
mod agent_boundary_tests {
    use super::{agent_route_allowed, static_asset, UI};

    #[test]
    fn the_agent_listener_has_no_wallet_or_mandate_creation_escape_hatch() {
        for (method, path) in [
            ("POST", "/api/export"),
            ("POST", "/api/export-file"),
            ("POST", "/api/import"),
            ("POST", "/api/send"),
            ("POST", "/api/withdraw"),
            ("POST", "/api/transfer"),
            ("POST", "/api/bind"),
            ("POST", "/api/settings"),
            ("POST", "/api/agent/mandates"),
            ("POST", "/api/raw"),
        ] {
            assert!(
                !agent_route_allowed(method, path),
                "{method} {path} escaped the allowlist"
            );
        }
        assert!(agent_route_allowed("POST", "/api/agent/execute-swap"));
        assert!(agent_route_allowed("POST", "/api/agent/pause"));
    }

    #[test]
    fn the_wallet_ui_loads_fonts_only_from_its_own_origin() {
        assert!(!UI.contains("fonts.googleapis.com"));
        assert!(!UI.contains("fonts.gstatic.com"));
        assert!(
            UI.contains("fonts/BricolageGrotesque.ttf"),
            "the wallet page must reference its own bundled font"
        );
        for path in [
            "/fonts/BricolageGrotesque.ttf",
            "/fonts/IBMPlexSans.ttf",
            "/fonts/IBMPlexMono-Regular.ttf",
            "/fonts/IBMPlexMono-Medium.ttf",
        ] {
            let (content_type, body) = static_asset(path).expect("font route");
            assert_eq!(content_type, "font/ttf");
            assert!(body.len() > 100_000, "{path} is not the bundled font");
        }
    }
}

#[cfg(test)]
mod origin_tests {
    use super::*;

    const OURS: &str = "chrome-extension://abcdefghijklmnopabcdefghijklmnop";
    const THEIRS: &str = "chrome-extension://zzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzz";

    /// With an allowlist set, only the named extension is answered. A hostile
    /// extension that asks for 127.0.0.1 permission gets no CORS headers, so
    /// the browser refuses to hand it the response.
    #[test]
    fn an_allowlist_admits_only_the_named_extension() {
        let allowed = vec![OURS.to_string()];
        assert!(cors_with(&Some(OURS.into()), &allowed).contains(OURS));
        assert_eq!(cors_with(&Some(THEIRS.into()), &allowed), "");
    }

    /// A trailing slash is the same origin, and should not be a way past the
    /// allowlist.
    #[test]
    fn a_trailing_slash_does_not_evade_the_allowlist() {
        let allowed = vec![OURS.to_string()];
        assert!(!cors_with(&Some(format!("{OURS}/")), &allowed).is_empty());
    }

    /// A page is never a cross-origin caller here, allowlist or not.
    #[test]
    fn a_web_page_is_never_answered() {
        for o in [
            "https://evil.example",
            "http://127.0.0.1:8977",
            "null",
            "chrome-extension:evil",
        ] {
            assert_eq!(cors_with(&Some(o.into()), &[]), "", "{o} must not be answered");
            assert_eq!(
                cors_with(&Some(o.into()), &[OURS.to_string()]),
                "",
                "{o} must not be answered"
            );
        }
    }

    /// Unset means every extension, which is the documented fallback — and
    /// the reason startup warns about it.
    #[test]
    fn an_empty_allowlist_falls_back_to_any_extension() {
        assert!(!cors_with(&Some(THEIRS.into()), &[]).is_empty());
    }

    /// The token comparison must not return early on the first wrong byte.
    #[test]
    fn the_agent_token_compares_in_constant_time() {
        assert!(secret_eq("abc123", "abc123"));
        assert!(!secret_eq("abc123", "abc124"));
        assert!(!secret_eq("abc123", "xbc123"));
        assert!(!secret_eq("abc123", "abc1234"));
        assert!(!secret_eq("", "x"));
        assert!(secret_eq("", ""));
    }
}

#[cfg(test)]
mod recovery_route_tests {
    use super::*;

    /// The routes that hand back or destroy recovery material.
    #[test]
    fn the_seed_routes_are_named() {
        for p in [
            "/api/export",
            "/api/export-file",
            "/api/reset",
            "/api/zyn-key",
            "/api/zyn-key/migrate",
        ] {
            assert!(same_origin_only(p), "{p} must be same-origin only");
        }
    }

    /// Ordinary routes are unaffected — the extension still drives the wallet.
    #[test]
    fn ordinary_routes_are_not_restricted() {
        for p in [
            "/api/overview",
            "/api/offers",
            "/api/offer",
            "/api/take",
            "/api/withdraw",
            "/api/transfer",
        ] {
            assert!(!same_origin_only(p), "{p} should stay reachable");
        }
    }

    /// `/api/export` is the one that returns the mnemonic, so if this list is
    /// ever trimmed, that is the entry whose loss matters most.
    #[test]
    fn export_is_covered() {
        assert!(same_origin_only("/api/export"));
    }
}
