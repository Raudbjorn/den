use super::{ConnectionError, Secret, auth_error, error, validate_key};
use crate::process::{self, Invocation};
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::{
    io::{Read, Write},
    net::{TcpListener, TcpStream},
    sync::atomic::{AtomicBool, Ordering},
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

const REDIRECT: &str = "http://localhost:43827/oauth/callback";
const LOGIN_TIMEOUT: Duration = Duration::from_secs(300);
const HTTP_TIMEOUT: Duration = Duration::from_secs(20);
const TOKEN_URL: &str = "https://api.linear.app/oauth/token";
const TOKEN_BODY_LIMIT: u64 = 32 * 1024;
const CALLBACK_LIMIT: u64 = 8 * 1024;

pub(super) fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}
pub(super) fn nonce() -> Result<String, ConnectionError> {
    let mut bytes = [0_u8; 32];
    getrandom::fill(&mut bytes).map_err(|_| error("Secure random generation failed"))?;
    Ok(URL_SAFE_NO_PAD.encode(bytes))
}
fn challenge(verifier: &str) -> String {
    URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()))
}
fn encode(value: &str) -> String {
    let mut output = String::new();
    for byte in value.bytes() {
        if byte.is_ascii_alphanumeric() || b"-._~".contains(&byte) {
            output.push(char::from(byte));
        } else {
            output.push('%');
            output.push(char::from(b"0123456789ABCDEF"[(byte >> 4) as usize]));
            output.push(char::from(b"0123456789ABCDEF"[(byte & 15) as usize]));
        }
    }
    output
}
fn form(fields: &[(&str, &str)]) -> String {
    fields
        .iter()
        .map(|(key, value)| format!("{}={}", encode(key), encode(value)))
        .collect::<Vec<_>>()
        .join("&")
}
fn decode(value: &str) -> Result<String, ConnectionError> {
    let mut bytes = Vec::new();
    let mut input = value.bytes();
    while let Some(byte) = input.next() {
        match byte {
            b'+' => bytes.push(b' '),
            b'%' => {
                let high = input
                    .next()
                    .and_then(|v| char::from(v).to_digit(16))
                    .ok_or_else(|| error("Invalid OAuth callback encoding"))?;
                let low = input
                    .next()
                    .and_then(|v| char::from(v).to_digit(16))
                    .ok_or_else(|| error("Invalid OAuth callback encoding"))?;
                bytes.push((high * 16 + low) as u8);
            }
            other => bytes.push(other),
        }
    }
    String::from_utf8(bytes).map_err(|_| error("Invalid OAuth callback encoding"))
}
fn callback_code(target: &str, state: &str) -> Result<Option<String>, ConnectionError> {
    let Some(query) = target.strip_prefix("/oauth/callback?") else {
        return Ok(None);
    };
    let mut fields = std::collections::BTreeMap::new();
    for part in query.split('&') {
        let (key, value) = part
            .split_once('=')
            .ok_or_else(|| error("Invalid OAuth callback"))?;
        if fields.insert(decode(key)?, decode(value)?).is_some() {
            return Err(error("Duplicate OAuth callback parameter"));
        }
    }
    if fields.get("state").map(String::as_str) != Some(state) {
        return Err(error(
            "OAuth callback state did not match; login was rejected",
        ));
    }
    if fields.contains_key("error") {
        return Err(error("Linear login was declined or canceled"));
    }
    fields
        .remove("code")
        .filter(|s| !s.is_empty() && s.len() <= 4096)
        .map(Some)
        .ok_or_else(|| error("OAuth callback omitted its authorization code"))
}
fn receive(
    stream: &mut TcpStream,
    state: &str,
    login_deadline: Instant,
) -> Result<Option<String>, ConnectionError> {
    let deadline = login_deadline.min(Instant::now() + Duration::from_secs(2));
    stream
        .set_write_timeout(Some(Duration::from_secs(2)))
        .map_err(|_| error("Could not configure OAuth callback"))?;
    let mut bytes = Vec::new();
    loop {
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            return Err(error("OAuth callback timed out"));
        }
        // A per-read timeout alone lets slow input extend the callback forever.
        stream
            .set_read_timeout(Some(remaining))
            .map_err(|_| error("Could not configure OAuth callback"))?;
        let mut chunk = [0_u8; 1024];
        let count = stream.read(&mut chunk).map_err(|error_value| {
            if matches!(
                error_value.kind(),
                std::io::ErrorKind::TimedOut | std::io::ErrorKind::WouldBlock
            ) {
                error("OAuth callback timed out")
            } else {
                error("Could not read OAuth callback")
            }
        })?;
        if count == 0 {
            return Err(error("Incomplete OAuth callback request"));
        }
        let newline = chunk[..count].iter().position(|byte| *byte == b'\n');
        bytes.extend_from_slice(&chunk[..newline.map_or(count, |position| position + 1)]);
        if bytes.len() as u64 > CALLBACK_LIMIT {
            return Err(error("OAuth callback exceeded its size limit"));
        }
        if newline.is_some() {
            break;
        }
    }
    let request =
        std::str::from_utf8(&bytes).map_err(|_| error("Invalid OAuth callback request"))?;
    let mut words = request.split_whitespace();
    if words.next() != Some("GET") {
        return Err(error("Unexpected OAuth callback method"));
    }
    let target = words
        .next()
        .ok_or_else(|| error("Invalid OAuth callback request"))?;
    let result = callback_code(target, state);
    let (status, message) = match &result {
        Ok(Some(_)) => (
            "200 OK",
            "Authorization received. Return to den to finish connecting.",
        ),
        Ok(None) => ("404 Not Found", "Not found."),
        Err(_) => (
            "400 Bad Request",
            "Authorization rejected. Return to den and try again.",
        ),
    };
    let response = format!(
        "HTTP/1.1 {status}\r\nContent-Type: text/plain\r\nCache-Control: no-store\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{message}",
        message.len()
    );
    stream
        .write_all(response.as_bytes())
        .map_err(|_| error("Could not answer OAuth callback"))?;
    result
}
pub(super) fn login(client: &str, cancel: &AtomicBool) -> Result<Secret, ConnectionError> {
    if cancel.load(Ordering::Acquire) {
        return Err(error("Linear login canceled"));
    }
    let listener=TcpListener::bind((std::net::Ipv4Addr::LOCALHOST,43827)).map_err(|_|error("Browser login needs localhost port 43827, which is unavailable; use an API key or retry"))?;
    listener
        .set_nonblocking(true)
        .map_err(|_| error("Could not configure browser login"))?;
    let verifier = nonce()?;
    let state = nonce()?;
    let challenge = challenge(&verifier);
    let url = format!(
        "https://linear.app/oauth/authorize?{}",
        form(&[
            ("client_id", client),
            ("redirect_uri", REDIRECT),
            ("response_type", "code"),
            ("scope", "read,write"),
            ("actor", "user"),
            ("state", &state),
            ("code_challenge", &challenge),
            ("code_challenge_method", "S256")
        ])
    );
    let deadline = Instant::now() + LOGIN_TIMEOUT;
    let opened = process::run_invocation(
        &Invocation::new("xdg-open").arg(url).discard_output(),
        &[],
        HTTP_TIMEOUT,
    )
    .map_err(|_| error("Could not open a browser; use an API key"))?;
    if !opened.status.success() {
        return Err(error("Could not open a browser; use an API key"));
    }
    let code = loop {
        if cancel.load(Ordering::Acquire) {
            return Err(error("Linear login canceled"));
        }
        if Instant::now() >= deadline {
            return Err(error("Linear login timed out after five minutes"));
        }
        match listener.accept() {
            Ok((mut stream, peer)) => {
                if !peer.ip().is_loopback() {
                    continue;
                }
                if let Some(code) = receive(&mut stream, &state, deadline)? {
                    break code;
                }
            }
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                std::thread::sleep(Duration::from_millis(50))
            }
            Err(_) => return Err(error("OAuth callback listener failed")),
        }
    };
    if cancel.load(Ordering::Acquire) {
        return Err(error("Linear login canceled"));
    }
    exchange(&[
        ("client_id", client),
        ("redirect_uri", REDIRECT),
        ("code", &code),
        ("code_verifier", &verifier),
        ("grant_type", "authorization_code"),
    ])
}
pub(super) fn refresh(client: &str, secret: &Secret) -> Result<Secret, ConnectionError> {
    let refresh = secret
        .refresh_token
        .as_deref()
        .ok_or_else(|| auth_error("Linear refresh token is unavailable; reconnect"))?;
    exchange(&[
        ("client_id", client),
        ("refresh_token", refresh),
        ("grant_type", "refresh_token"),
    ])
}
fn exchange(fields: &[(&str, &str)]) -> Result<Secret, ConnectionError> {
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .timeout_global(Some(HTTP_TIMEOUT))
        .http_status_as_error(false)
        .https_only(true)
        .max_redirects(0)
        .build()
        .into();
    let response = agent
        .post(TOKEN_URL)
        .header("Content-Type", "application/x-www-form-urlencoded")
        .send(form(fields).as_bytes())
        .map_err(|_| error("Could not exchange Linear authorization; retry login"))?;
    if !response.status().is_success() {
        return Err(match response.status().as_u16() {
            400 | 401 | 403 => auth_error("Linear rejected OAuth authorization; reconnect"),
            429 => error("Linear authorization is rate limited; retry later"),
            _ => error("Linear authorization service is unavailable; retry later"),
        });
    }
    let bytes = response
        .into_body()
        .into_with_config()
        .limit(TOKEN_BODY_LIMIT)
        .read_to_vec()
        .map_err(|_| error("Could not read Linear authorization response"))?;
    parse_token(&bytes, now())
}
fn parse_token(bytes: &[u8], now: u64) -> Result<Secret, ConnectionError> {
    #[derive(Deserialize)]
    struct Token {
        access_token: String,
        refresh_token: String,
        expires_in: u64,
        token_type: String,
    }
    let token: Token = serde_json::from_slice(bytes)
        .map_err(|_| error("Linear returned incomplete OAuth credentials"))?;
    validate_key(&token.access_token)?;
    validate_key(&token.refresh_token)?;
    if !token.token_type.eq_ignore_ascii_case("bearer") || token.expires_in == 0 {
        return Err(error("Linear returned invalid OAuth credentials"));
    }
    Ok(Secret {
        access_token: token.access_token,
        refresh_token: Some(token.refresh_token),
        expires_at: Some(now.saturating_add(token.expires_in)),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn pkce_matches_rfc7636_vector() {
        assert_eq!(
            challenge("dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk"),
            "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM"
        );
    }
    #[test]
    fn callback_rejects_wrong_missing_and_duplicate_state() {
        for target in [
            "/oauth/callback?code=a&state=wrong",
            "/oauth/callback?code=a",
            "/oauth/callback?code=a&state=s&state=s",
        ] {
            assert!(callback_code(target, "s").is_err());
        }
        assert_eq!(
            callback_code("/oauth/callback?code=a%2Bb&state=s", "s").unwrap(),
            Some("a+b".into())
        );
        assert!(callback_code("/oauth/callback?error=access_denied&state=s", "s").is_err());
        assert_eq!(callback_code("/favicon.ico", "s").unwrap(), None);
    }
    #[test]
    fn form_encoding_preserves_arbitrary_parameter_values() {
        for s in ["a+b&c=d", "https://localhost/a?b=c", "Ísland"] {
            assert_eq!(decode(&encode(s)).unwrap(), s);
        }
        assert!(decode("%Q1").is_err());
    }
    #[test]
    fn token_response_requires_rotating_pair_and_tracks_expiration() {
        let secret=parse_token(br#"{"access_token":"access","refresh_token":"refresh","expires_in":3600,"token_type":"Bearer"}"#,100).unwrap();
        assert_eq!(secret.expires_at, Some(3700));
        assert!(
            parse_token(
                br#"{"access_token":"private","expires_in":10,"token_type":"Bearer"}"#,
                0
            )
            .is_err()
        );
    }
    #[test]
    fn partial_callback_obeys_the_absolute_login_deadline() {
        let listener = TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0)).unwrap();
        let mut client = TcpStream::connect(listener.local_addr().unwrap()).unwrap();
        let (mut server, _) = listener.accept().unwrap();
        client.write_all(b"GET /oauth/callback?").unwrap();
        let started = Instant::now();
        let failure =
            receive(&mut server, "state", started + Duration::from_millis(25)).unwrap_err();
        assert!(failure.to_string().contains("timed out"));
        assert!(
            started.elapsed() < Duration::from_secs(1),
            "the 2s per-read timeout must not extend the login deadline"
        );
    }
}
