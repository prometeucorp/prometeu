//! Shared OAuth mechanics for Linear and discovered MCP servers: PKCE, browser authorization,
//! loopback callbacks, and token exchange. Return structured failure reasons for each integration
//! to translate. Completion-page text comes from the caller because the browser cannot use the
//! frontend catalog.

use base64::Engine;
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

/// Structured callback failure reasons translated by each integration.
#[derive(Debug)]
pub enum Denied {
    /// The person denied consent.
    Refused,
    /// The authorization server returned another error; retain its diagnostic.
    Error(String),
    /// The callback contained neither code nor error.
    NoCode,
    /// The authorization deadline expired.
    Timeout,
    /// The callback socket failed.
    Broken(String),
}

/// Localized completion-page title and message supplied by the caller.
pub type Page = dyn Fn(bool, &str) -> String;

pub fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// Generate 64 hexadecimal characters from two secure UUIDs for state and an RFC 7636 verifier.
pub fn random() -> String {
    format!(
        "{}{}",
        uuid::Uuid::new_v4().simple(),
        uuid::Uuid::new_v4().simple()
    )
}

/// Compute the S256 challenge as unpadded base64url of the verifier's SHA-256 hash.
pub fn challenge(verifier: &str) -> String {
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()))
}

/// Encode OAuth form bodies without enabling reqwest's additional form feature.
pub fn form(fields: &[(&str, &str)]) -> String {
    fields
        .iter()
        .map(|(k, v)| format!("{}={}", escape(k), escape(v)))
        .collect::<Vec<_>>()
        .join("&")
}

/// Percent-encode URL and form values, including redirect URIs; hexadecimal and base64url values
/// remain unchanged.
pub fn escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len() * 3);
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char)
            }
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

pub fn unescape(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'+' => out.push(b' '),
            b'%' if i + 2 < bytes.len() => {
                let hex = std::str::from_utf8(&bytes[i + 1..i + 3]).ok();
                match hex.and_then(|h| u8::from_str_radix(h, 16).ok()) {
                    Some(byte) => {
                        out.push(byte);
                        i += 2;
                    }
                    None => out.push(bytes[i]),
                }
            }
            other => out.push(other),
        }
        i += 1;
    }
    String::from_utf8_lossy(&out).to_string()
}

pub fn parse_query(q: &str) -> HashMap<String, String> {
    q.split('&')
        .filter(|p| !p.is_empty())
        .map(|p| {
            let (k, v) = p.split_once('=').unwrap_or((p, ""));
            (unescape(k), unescape(v))
        })
        .collect()
}

/// Parse the path from a GET request line; other methods are not valid authorization callbacks.
pub fn request_target(req: &str) -> Option<String> {
    let mut words = req.lines().next()?.split_whitespace();
    let method = words.next()?;
    let target = words.next()?;
    (method == "GET").then(|| target.to_string())
}

const REQUEST_LINE_LIMIT: usize = 8 * 1024;
const HEADER_LIMIT: usize = 256 * 1024;

/// Keep only the request line, then drain the remaining headers so closing the socket does not reset
/// the browser's connection. Loopback cookies are shared across ports and can make headers large.
fn callback_target(stream: &mut TcpStream, deadline: Instant) -> std::io::Result<Option<String>> {
    let mut chunk = [0; 8192];
    let mut line = Vec::new();
    let mut target = None;
    let mut tail = Vec::new();
    let mut total = 0;
    while total < HEADER_LIMIT {
        let remaining = deadline
            .checked_duration_since(Instant::now())
            .filter(|duration| !duration.is_zero())
            .ok_or_else(|| {
                std::io::Error::new(std::io::ErrorKind::TimedOut, "callback headers timed out")
            })?;
        stream.set_read_timeout(Some(remaining))?;
        let read = stream.read(&mut chunk)?;
        if read == 0 {
            return Ok(None);
        }
        total += read;
        if target.is_none() {
            line.extend_from_slice(&chunk[..read]);
            let Some(end) = line.windows(2).position(|bytes| bytes == b"\r\n") else {
                if line.len() >= REQUEST_LINE_LIMIT {
                    return Ok(None);
                }
                continue;
            };
            target = Some(request_target(&String::from_utf8_lossy(&line[..end])));
            tail = line.split_off(end);
        } else {
            tail.extend_from_slice(&chunk[..read]);
        }
        if tail.windows(4).any(|bytes| bytes == b"\r\n\r\n") {
            return Ok(target.flatten());
        }
        tail.drain(..tail.len().saturating_sub(3));
    }
    Ok(None)
}

/// Wait for the callback until the deadline. Return 404 for unrelated paths, such as favicon.ico,
/// and continue waiting.
pub fn wait_for_code(
    listener: &TcpListener,
    route: &str,
    state: &str,
    deadline: Instant,
    page: &Page,
) -> Result<String, Denied> {
    loop {
        if Instant::now() >= deadline {
            return Err(Denied::Timeout);
        }
        match listener.accept() {
            Ok((mut stream, _)) => {
                let _ = stream.set_nonblocking(false);
                let _ = stream.set_write_timeout(Some(Duration::from_secs(2)));
                let request_deadline = deadline.min(Instant::now() + Duration::from_secs(2));
                let Some(target) = callback_target(&mut stream, request_deadline)
                    .ok()
                    .flatten()
                else {
                    respond(&mut stream, "404 Not Found", "");
                    continue;
                };
                let (path, query) = target.split_once('?').unwrap_or((&target, ""));
                if path != route {
                    respond(&mut stream, "404 Not Found", "");
                    continue;
                }
                let q = parse_query(query);
                // Validate state before accepting even an error response, preventing unrelated
                // local requests from cancelling an active login.
                if q.get("state").map(String::as_str) != Some(state) {
                    respond(&mut stream, "400 Bad Request", &page(false, ""));
                    continue;
                }
                if let Some(err) = q.get("error") {
                    let why = q.get("error_description").cloned().unwrap_or_default();
                    respond(&mut stream, "200 OK", &page(false, &why));
                    return Err(match err.as_str() {
                        "access_denied" => Denied::Refused,
                        _ => Denied::Error(format!("{err} {why}").trim().to_string()),
                    });
                }
                let Some(code) = q.get("code").filter(|c| !c.is_empty()) else {
                    respond(&mut stream, "400 Bad Request", &page(false, ""));
                    return Err(Denied::NoCode);
                };
                respond(&mut stream, "200 OK", &page(true, ""));
                return Ok(code.clone());
            }
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                if Instant::now() > deadline {
                    return Err(Denied::Timeout);
                }
                std::thread::sleep(Duration::from_millis(100));
            }
            Err(e) => return Err(Denied::Broken(e.to_string())),
        }
    }
}

pub fn respond(stream: &mut TcpStream, status: &str, body: &str) {
    let _ = write!(
        stream,
        "HTTP/1.1 {status}\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\n\
         Content-Security-Policy: default-src 'none'; style-src 'unsafe-inline'; base-uri 'none'; frame-ancestors 'none'\r\n\
         X-Content-Type-Options: nosniff\r\nReferrer-Policy: no-referrer\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    let _ = stream.flush();
}

/// Serve the completion page using the caller's localized title and message.
pub fn page(lang: &str, title: &str, text: &str) -> String {
    let lang = html(lang);
    let title = html(title);
    let text = html(text);
    format!(
        "<!doctype html><html lang=\"{lang}\"><meta charset=\"utf-8\"><title>{title}</title>\
         <body style=\"margin:0;height:100vh;display:grid;place-items:center;background:#141110;\
         color:#eae8e6;font:16px/1.5 -apple-system,system-ui,sans-serif\">\
         <div style=\"text-align:center\"><div style=\"font-size:22px;font-weight:600\">{title}</div>\
         <div style=\"color:#a4a09d;margin-top:8px\">{text}</div></div></body></html>"
    )
}

pub fn html(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#39;")
}

#[cfg(test)]
mod tests {
    use super::*;

    /// RFC 7636 Appendix B test vector.
    #[test]
    fn challenge_matches_the_rfc_example() {
        assert_eq!(
            challenge("dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk"),
            "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM"
        );
    }

    #[test]
    fn query_is_decoded() {
        let q = parse_query("code=abc%20def&state=xyz");
        assert_eq!(q.get("code").map(String::as_str), Some("abc def"));
        assert_eq!(q.get("state").map(String::as_str), Some("xyz"));
    }

    #[test]
    fn only_get_requests_supply_a_target() {
        assert_eq!(
            request_target("GET /mcp?code=1 HTTP/1.1\r\n"),
            Some("/mcp?code=1".to_string())
        );
        assert_eq!(request_target("POST /mcp HTTP/1.1\r\n"), None);
    }
}

/// Public consent data; the verifier and provider credentials stay in the execution host.
#[derive(Clone, serde::Serialize, serde::Deserialize)]
pub struct ConsentRequest {
    pub authorize: String,
    pub state: String,
    #[serde(default)]
    pub language: String,
}
/// Host-specific browser and loopback handling. Implementations bind before opening the browser.
pub trait Consent: Send + Sync {
    fn authorize(&self, request: &ConsentRequest) -> Result<String, String>;
}
pub const MCP_REDIRECT: &str = "http://127.0.0.1:17421/mcp";
pub const MCP_PORT: u16 = 17421;
pub const MCP_ROUTE: &str = "/mcp";

pub fn mcp_page(language: &str, ok: bool, why: &str) -> String {
    let (title, text) = match (ok, language == "pt") {
        (true, true) => (
            "Conectado",
            "Pode fechar esta aba e voltar ao Prometeu.".to_string(),
        ),
        (true, false) => (
            "Connected",
            "You can close this tab and go back to Prometeu.".to_string(),
        ),
        (false, true) => (
            "Não deu",
            format!("O servidor não autorizou o Prometeu. {why}")
                .trim()
                .to_string(),
        ),
        (false, false) => (
            "Did not work",
            format!("The server did not authorize Prometeu. {why}")
                .trim()
                .to_string(),
        ),
    };
    page(language, title, &text)
}

#[cfg(test)]
mod callback_tests {
    use super::*;
    #[test]
    fn callback_rejects_truncated_and_oversized_headers() {
        for headers in [
            b"GET /mcp?state=expected&code=partial".to_vec(),
            vec![b'x'; REQUEST_LINE_LIMIT],
            [b"GET /mcp HTTP/1.1\r\n".to_vec(), vec![b'x'; HEADER_LIMIT]].concat(),
        ] {
            let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
            let address = listener.local_addr().unwrap();
            let task = std::thread::spawn(move || {
                let (mut stream, _) = listener.accept().unwrap();
                callback_target(&mut stream, Instant::now() + Duration::from_secs(2))
            });
            let mut stream = TcpStream::connect(address).unwrap();
            stream.write_all(&headers).unwrap();
            stream.shutdown(std::net::Shutdown::Write).unwrap();
            assert!(task.join().unwrap().unwrap().is_none());
        }
    }

    #[test]
    fn callback_waits_for_fragmented_headers_before_accepting_code() {
        let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        listener.set_nonblocking(true).unwrap();
        let address = listener.local_addr().unwrap();
        let task = std::thread::spawn(move || {
            wait_for_code(
                &listener,
                "/mcp",
                "expected",
                Instant::now() + Duration::from_secs(3),
                &|_, _| String::new(),
            )
        });
        let mut stream = TcpStream::connect(address).unwrap();
        stream
            .write_all(b"GET /mcp?state=expected&code=par")
            .unwrap();
        stream
            .set_read_timeout(Some(Duration::from_millis(150)))
            .unwrap();
        let error = stream.read(&mut [0; 1]).unwrap_err();
        assert!(matches!(
            error.kind(),
            std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
        ));
        stream
            .write_all(b"tial%20code HTTP/1.1\r\nHost: localhost\r\n\r\n")
            .unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(3)))
            .unwrap();
        let mut response = String::new();
        stream.read_to_string(&mut response).unwrap();
        assert!(response.starts_with("HTTP/1.1 200"));
        assert_eq!(task.join().unwrap().unwrap(), "partial code");
    }

    #[test]
    fn callback_accepts_code_behind_large_loopback_cookies() {
        let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let address = listener.local_addr().unwrap();
        let task = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            callback_target(&mut stream, Instant::now() + Duration::from_secs(2))
        });
        let mut stream = TcpStream::connect(address).unwrap();
        let cookie = "x".repeat(32 * 1024);
        write!(
            stream,
            "GET /mcp?state=expected&code=ok HTTP/1.1\r\nCookie: a={cookie}\r\n\r\n"
        )
        .unwrap();
        assert_eq!(
            task.join().unwrap().unwrap().as_deref(),
            Some("/mcp?state=expected&code=ok")
        );
    }

    #[test]
    fn callback_ignores_wrong_state_and_unrelated_requests_then_decodes_code() {
        let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        listener.set_nonblocking(true).unwrap();
        let address = listener.local_addr().unwrap();
        let task = std::thread::spawn(move || {
            wait_for_code(
                &listener,
                "/mcp",
                "expected",
                Instant::now() + Duration::from_secs(3),
                &|_, _| String::new(),
            )
        });
        for (target, status) in [
            ("/favicon.ico", "404"),
            ("/mcp?state=wrong&error=access_denied", "400"),
            ("/mcp?state=expected&code=approved%20code", "200"),
        ] {
            let mut stream = TcpStream::connect(address).unwrap();
            stream
                .set_read_timeout(Some(Duration::from_secs(3)))
                .unwrap();
            write!(stream, "GET {target} HTTP/1.1\r\nHost: localhost\r\n\r\n").unwrap();
            let mut response = String::new();
            stream.read_to_string(&mut response).unwrap();
            assert!(response.starts_with(&format!("HTTP/1.1 {status}")));
        }
        assert_eq!(task.join().unwrap().unwrap(), "approved code");
    }
    #[test]
    fn abandoned_consent_expires() {
        let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        listener.set_nonblocking(true).unwrap();
        assert!(matches!(
            wait_for_code(&listener, "/mcp", "state", Instant::now(), &|_, _| {
                String::new()
            }),
            Err(Denied::Timeout)
        ));
    }
}
