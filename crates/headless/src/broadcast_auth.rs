//! Access-token check for `camouflage-broadcast`'s WebSocket handshake.
//!
//! The broadcast server replays the whole session (prompts, model output,
//! tool stdout, patches) to every client that connects. Browsers let any web
//! page open a WebSocket to `ws://localhost:<port>`, and the viewer is meant
//! to be hosted on another origin (GitHub Pages, S3), so an Origin allowlist
//! alone can't tell the viewer apart from a hostile page. Instead every
//! client must present a random access token that is only printed to the
//! operator's terminal:
//!
//! - as a `token` query parameter (`ws://host:port/?token=…`), which is what
//!   browsers can send, or
//! - as an `Authorization: Bearer …` header, for non-browser clients.
//!
//! Anything else is refused with `401 Unauthorized` before the upgrade.

use tokio_tungstenite::tungstenite::handshake::server::{ErrorResponse, Request, Response};
use tokio_tungstenite::tungstenite::http::{header, StatusCode};
use uuid::Uuid;

/// Query parameter that carries the access token.
pub const TOKEN_PARAM: &str = "token";

/// Environment variable `camouflage-broadcast` reads a fixed token from.
pub const TOKEN_ENV: &str = "CAMOUFLAGE_BROADCAST_TOKEN";

/// A fresh random access token: 64 hex characters drawn from the OS CSPRNG
/// (two v4 UUIDs, 244 random bits).
pub fn generate_token() -> String {
    format!("{}{}", Uuid::new_v4().simple(), Uuid::new_v4().simple())
}

/// Compare two byte strings without exiting early on the first mismatch, so
/// response timing doesn't reveal how much of a guessed token was right.
/// (A length mismatch returns immediately; the length isn't secret.)
pub fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

/// The (percent-decoded) value of the `token` query parameter, if present.
pub fn token_from_query(query: Option<&str>) -> Option<String> {
    query?.split('&').find_map(|pair| {
        let (key, value) = pair.split_once('=').unwrap_or((pair, ""));
        (percent_decode(key)? == TOKEN_PARAM).then(|| percent_decode(value))?
    })
}

/// `application/x-www-form-urlencoded` decoding (`+` → space, `%XX` → byte).
/// Returns `None` for malformed escapes or non-UTF-8 output.
fn percent_decode(s: &str) -> Option<String> {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'+' => out.push(b' '),
            b'%' => {
                let hex = s.get(i + 1..i + 3)?;
                out.push(u8::from_str_radix(hex, 16).ok()?);
                i += 2;
            }
            b => out.push(b),
        }
        i += 1;
    }
    String::from_utf8(out).ok()
}

/// The token from an `Authorization: Bearer <token>` header, if present.
fn bearer_token(req: &Request) -> Option<&str> {
    let value = req.headers().get(header::AUTHORIZATION)?.to_str().ok()?;
    let (scheme, token) = value.trim().split_once(' ')?;
    scheme.eq_ignore_ascii_case("bearer").then(|| token.trim())
}

/// Whether the handshake request carries `expected` as its access token.
pub fn is_authorized(req: &Request, expected: &str) -> bool {
    let expected = expected.as_bytes();
    if let Some(token) = token_from_query(req.uri().query()) {
        if constant_time_eq(token.as_bytes(), expected) {
            return true;
        }
    }
    bearer_token(req).is_some_and(|token| constant_time_eq(token.as_bytes(), expected))
}

/// The `401 Unauthorized` sent to clients without a valid token.
pub fn unauthorized() -> ErrorResponse {
    let mut resp = ErrorResponse::new(Some(
        "camouflage-broadcast: missing or invalid access token \
         (connect with ?token=<token> as printed at startup)\n"
            .to_string(),
    ));
    *resp.status_mut() = StatusCode::UNAUTHORIZED;
    resp.headers_mut().insert(
        header::CONTENT_TYPE,
        header::HeaderValue::from_static("text/plain; charset=utf-8"),
    );
    resp
}

/// Handshake callback body for `tokio_tungstenite::accept_hdr_async`:
/// accept the upgrade only when the request carries the access token.
// The large `Err` type is fixed by tungstenite's `Callback` signature.
#[allow(clippy::result_large_err)]
pub fn check_handshake(
    req: &Request,
    resp: Response,
    expected: &str,
) -> Result<Response, ErrorResponse> {
    if is_authorized(req, expected) {
        Ok(resp)
    } else {
        Err(unauthorized())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const TOKEN: &str = "s3cr3t-token";

    fn req(uri: &str) -> Request {
        Request::builder().uri(uri).body(()).unwrap()
    }

    fn req_with_auth(uri: &str, auth: &str) -> Request {
        Request::builder()
            .uri(uri)
            .header(header::AUTHORIZATION, auth)
            .body(())
            .unwrap()
    }

    #[test]
    fn generated_tokens_are_long_hex_and_unique() {
        let a = generate_token();
        let b = generate_token();
        assert_eq!(a.len(), 64);
        assert!(a.bytes().all(|c| c.is_ascii_hexdigit()));
        assert_ne!(a, b);
    }

    #[test]
    fn constant_time_eq_matches_only_identical_input() {
        assert!(constant_time_eq(b"abc", b"abc"));
        assert!(!constant_time_eq(b"abc", b"abd"));
        assert!(!constant_time_eq(b"abc", b"ab"));
        assert!(!constant_time_eq(b"", b"a"));
        assert!(constant_time_eq(b"", b""));
    }

    #[test]
    fn query_token_is_extracted_and_decoded() {
        assert_eq!(token_from_query(Some("token=abc")).as_deref(), Some("abc"));
        assert_eq!(
            token_from_query(Some("x=1&token=abc&y=2")).as_deref(),
            Some("abc")
        );
        assert_eq!(
            token_from_query(Some("token=a%2Bb+c")).as_deref(),
            Some("a+b c")
        );
        assert_eq!(
            token_from_query(Some("tok%65n=abc")).as_deref(),
            Some("abc")
        );
        assert_eq!(token_from_query(Some("tokens=abc")), None);
        assert_eq!(token_from_query(Some("token=%zz")), None);
        assert_eq!(token_from_query(Some("token=%4")), None);
        assert_eq!(token_from_query(Some("")), None);
        assert_eq!(token_from_query(None), None);
    }

    #[test]
    fn accepts_matching_query_token() {
        assert!(is_authorized(&req("/?token=s3cr3t-token"), TOKEN));
        assert!(is_authorized(&req("/?foo=bar&token=s3cr3t-token"), TOKEN));
        assert!(is_authorized(&req("/?token=s3cr3t%2Dtoken"), TOKEN));
    }

    #[test]
    fn accepts_matching_bearer_header() {
        assert!(is_authorized(
            &req_with_auth("/", "Bearer s3cr3t-token"),
            TOKEN
        ));
        assert!(is_authorized(
            &req_with_auth("/", "bearer  s3cr3t-token "),
            TOKEN
        ));
    }

    #[test]
    fn rejects_missing_or_wrong_token() {
        assert!(!is_authorized(&req("/"), TOKEN));
        assert!(!is_authorized(&req("/?token="), TOKEN));
        assert!(!is_authorized(&req("/?token=s3cr3t-tokeN"), TOKEN));
        assert!(!is_authorized(&req("/?token=s3cr3t-token-and-more"), TOKEN));
        assert!(!is_authorized(&req("/?token=s3cr3t"), TOKEN));
        assert!(!is_authorized(&req("/s3cr3t-token"), TOKEN));
        assert!(!is_authorized(
            &req_with_auth("/", "Basic s3cr3t-token"),
            TOKEN
        ));
        assert!(!is_authorized(&req_with_auth("/", "Bearer nope"), TOKEN));
    }

    #[test]
    fn check_handshake_rejects_with_401() {
        let err = check_handshake(&req("/?token=nope"), Response::new(()), TOKEN).unwrap_err();
        assert_eq!(err.status(), StatusCode::UNAUTHORIZED);
        assert!(check_handshake(&req("/?token=s3cr3t-token"), Response::new(()), TOKEN).is_ok());
    }
}
