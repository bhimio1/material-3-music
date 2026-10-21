//! Signing in to YouTube Music, the two ways ytmusicapi supports:
//!
//! - **OAuth device flow** with the user's own Google Cloud client
//!   ("TVs and Limited Input devices"). Requests carry a bearer token that
//!   is refreshed automatically.
//! - **Browser session**: the Cookie header copied from music.youtube.com.
//!   Requests carry the cookies and a `SAPISIDHASH` authorization derived
//!   from the `SAPISID` cookie.
//!
//! Credentials live in `~/.local/share/m3-music/auth.json`, readable only
//! by the user.

use anyhow::{anyhow, bail, Context, Result};
use serde::{Deserialize, Serialize};
use sha1::{Digest, Sha1};
use std::path::PathBuf;

pub const ORIGIN: &str = "https://music.youtube.com";
const DEVICE_CODE_URL: &str = "https://oauth2.googleapis.com/device/code";
const TOKEN_URL: &str = "https://oauth2.googleapis.com/token";
const SCOPE: &str = "https://www.googleapis.com/auth/youtube";

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum Auth {
    OAuth {
        client_id: String,
        client_secret: String,
        access_token: String,
        refresh_token: String,
        /// Unix seconds.
        expires_at: i64,
    },
    Browser {
        cookie: String,
        #[serde(default)]
        auth_user: String,
    },
}

fn now() -> i64 {
    chrono::Utc::now().timestamp()
}

pub fn data_dir() -> PathBuf {
    dirs::data_dir()
        .unwrap_or_else(|| material_3_ui::sys::paths::home().join(".local/share"))
        .join("m3-music")
}

/// Write a file only the user can read.
pub fn write_private(path: &std::path::Path, contents: &[u8]) -> std::io::Result<()> {
    use std::io::Write;
    use std::os::unix::fs::OpenOptionsExt;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let tmp = path.with_extension("tmp");
    let mut f = std::fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(0o600)
        .open(&tmp)?;
    f.write_all(contents)?;
    f.sync_all()?;
    std::fs::rename(tmp, path)
}

impl Auth {
    pub fn path() -> PathBuf {
        data_dir().join("auth.json")
    }

    pub fn load() -> Option<Auth> {
        let text = std::fs::read_to_string(Self::path()).ok()?;
        serde_json::from_str(&text).ok()
    }

    pub fn save(&self) -> Result<()> {
        let json = serde_json::to_vec_pretty(self)?;
        write_private(&Self::path(), &json).context("couldn't save credentials")
    }

    pub fn delete() {
        let _ = std::fs::remove_file(Self::path());
        let _ = std::fs::remove_file(cookies_path());
    }

    pub fn label(&self) -> &'static str {
        match self {
            Auth::OAuth { .. } => "Google account (OAuth)",
            Auth::Browser { .. } => "Browser session",
        }
    }

    pub fn cookie(&self) -> Option<&str> {
        match self {
            Auth::Browser { cookie, .. } => Some(cookie),
            Auth::OAuth { .. } => None,
        }
    }

    /// Headers for an InnerTube request. Refreshes an expired OAuth token
    /// first (blocking), returning true in the second field when the
    /// credentials changed and should be saved.
    pub fn headers(&mut self, agent: &ureq::Agent) -> Result<(Vec<(String, String)>, bool)> {
        match self {
            Auth::OAuth {
                client_id,
                client_secret,
                access_token,
                refresh_token,
                expires_at,
            } => {
                let mut changed = false;
                if *expires_at - 60 <= now() {
                    let t = refresh(agent, client_id, client_secret, refresh_token)?;
                    *access_token = t.access_token;
                    *expires_at = now() + t.expires_in;
                    if let Some(r) = t.refresh_token {
                        *refresh_token = r;
                    }
                    changed = true;
                }
                Ok((
                    vec![
                        ("Authorization".into(), format!("Bearer {access_token}")),
                        ("X-Goog-Request-Time".into(), now().to_string()),
                    ],
                    changed,
                ))
            }
            Auth::Browser { cookie, auth_user } => {
                let sapisid = cookie_value(cookie, "__Secure-3PAPISID")
                    .or_else(|| cookie_value(cookie, "SAPISID"))
                    .ok_or_else(|| anyhow!("the cookie has no SAPISID; copy it again while signed in"))?;
                Ok((
                    vec![
                        ("Cookie".into(), cookie.clone()),
                        ("Authorization".into(), sapisid_hash(&sapisid, ORIGIN, now())),
                        ("X-Goog-AuthUser".into(), if auth_user.is_empty() { "0".into() } else { auth_user.clone() }),
                        ("X-Origin".into(), ORIGIN.into()),
                    ],
                    false,
                ))
            }
        }
    }
}

/// `SAPISIDHASH <ts>_<sha1("<ts> <SAPISID> <origin>")>`.
pub fn sapisid_hash(sapisid: &str, origin: &str, ts: i64) -> String {
    let digest = Sha1::digest(format!("{ts} {sapisid} {origin}").as_bytes());
    let hex: String = digest.iter().map(|b| format!("{b:02x}")).collect();
    format!("SAPISIDHASH {ts}_{hex}")
}

pub fn cookie_value(cookie: &str, name: &str) -> Option<String> {
    cookie.split(';').find_map(|pair| {
        let (k, v) = pair.trim().split_once('=')?;
        (k == name).then(|| v.to_string())
    })
}

/// Accepts what people copy from devtools: the whole request-headers block
/// (`name: value` lines, or Chrome's alternating name/value lines), or just
/// the cookie string.
pub fn parse_browser_input(input: &str) -> Result<Auth> {
    let input = input.trim();
    if input.is_empty() {
        bail!("paste the request headers or the Cookie value first");
    }
    let lines: Vec<&str> = input.lines().map(str::trim).filter(|l| !l.is_empty()).collect();
    let mut cookie = None;
    let mut auth_user = String::new();
    for (i, line) in lines.iter().enumerate() {
        let (name, value) = match line.split_once(": ") {
            Some((n, v)) => (n.trim_start_matches(':').to_string(), v.to_string()),
            // Chrome's "copy as list": name on one line, value on the next.
            None if line.ends_with(':') => (
                line.trim_end_matches(':').to_string(),
                lines.get(i + 1).copied().unwrap_or("").to_string(),
            ),
            None => continue,
        };
        match name.to_ascii_lowercase().as_str() {
            "cookie" => cookie = Some(value),
            "x-goog-authuser" => auth_user = value,
            _ => {}
        }
    }
    let cookie = match cookie {
        Some(c) => c,
        None if input.contains('=') && !input.contains('\n') => input.trim_start_matches("Cookie:").trim().to_string(),
        None => bail!("no Cookie header found in what was pasted"),
    };
    if cookie_value(&cookie, "SAPISID").is_none() && cookie_value(&cookie, "__Secure-3PAPISID").is_none() {
        bail!("the cookie has no SAPISID. Copy it from a request to music.youtube.com while signed in");
    }
    Ok(Auth::Browser { cookie, auth_user })
}

pub fn cookies_path() -> PathBuf {
    data_dir().join("cookies.txt")
}

/// Netscape cookies.txt for yt-dlp (playback and downloads of
/// members-only or age-restricted songs).
pub fn netscape_cookies(cookie: &str) -> String {
    let expiry = now() + 365 * 24 * 3600;
    let mut out = String::from("# Netscape HTTP Cookie File\n");
    for pair in cookie.split(';') {
        if let Some((k, v)) = pair.trim().split_once('=') {
            let secure = if k.starts_with("__Secure") { "TRUE" } else { "FALSE" };
            out.push_str(&format!(".youtube.com\tTRUE\t/\t{secure}\t{expiry}\t{k}\t{v}\n"));
        }
    }
    out
}

/// Write cookies.txt for the current browser session (or remove it).
pub fn sync_cookie_file(auth: Option<&Auth>) -> Option<PathBuf> {
    match auth.and_then(Auth::cookie) {
        Some(cookie) => {
            let path = cookies_path();
            write_private(&path, netscape_cookies(cookie).as_bytes()).ok()?;
            Some(path)
        }
        None => {
            let _ = std::fs::remove_file(cookies_path());
            None
        }
    }
}

#[derive(Clone, Debug, Deserialize)]
pub struct DeviceCode {
    pub device_code: String,
    pub user_code: String,
    #[serde(alias = "verification_uri")]
    pub verification_url: String,
    #[serde(default = "default_interval")]
    pub interval: u64,
    #[serde(default)]
    pub expires_in: u64,
}

fn default_interval() -> u64 {
    5
}

#[derive(Debug, Deserialize)]
struct TokenResponse {
    access_token: String,
    #[serde(default = "default_expiry")]
    expires_in: i64,
    refresh_token: Option<String>,
}

fn default_expiry() -> i64 {
    3600
}

fn form_post(agent: &ureq::Agent, url: &str, form: &[(&str, &str)]) -> Result<serde_json::Value> {
    match agent.post(url).send_form(form) {
        Ok(resp) => Ok(resp.into_json()?),
        // OAuth errors come back as 4xx with a JSON body.
        Err(ureq::Error::Status(_, resp)) => Ok(resp.into_json()?),
        Err(e) => Err(anyhow!(e)),
    }
}

pub fn start_device_flow(agent: &ureq::Agent, client_id: &str) -> Result<DeviceCode> {
    let v = form_post(agent, DEVICE_CODE_URL, &[("client_id", client_id), ("scope", SCOPE)])?;
    if let Some(err) = v.get("error").and_then(|e| e.as_str()) {
        let desc = v.get("error_description").and_then(|e| e.as_str()).unwrap_or("");
        bail!("Google refused the client ({err}). {desc}");
    }
    Ok(serde_json::from_value(v)?)
}

pub enum Poll {
    Pending,
    SlowDown,
    Done(Auth),
    Failed(String),
}

pub fn poll_device(agent: &ureq::Agent, client_id: &str, client_secret: &str, device_code: &str) -> Poll {
    let v = match form_post(
        agent,
        TOKEN_URL,
        &[
            ("client_id", client_id),
            ("client_secret", client_secret),
            ("device_code", device_code),
            ("grant_type", "urn:ietf:params:oauth:grant-type:device_code"),
        ],
    ) {
        Ok(v) => v,
        Err(e) => return Poll::Failed(e.to_string()),
    };
    match v.get("error").and_then(|e| e.as_str()) {
        Some("authorization_pending") => Poll::Pending,
        Some("slow_down") => Poll::SlowDown,
        Some("access_denied") => Poll::Failed("Sign-in was cancelled.".into()),
        Some("expired_token") => Poll::Failed("The code expired. Start again.".into()),
        Some(other) => Poll::Failed(format!("Google returned {other}")),
        None => match serde_json::from_value::<TokenResponse>(v) {
            Ok(t) => Poll::Done(Auth::OAuth {
                client_id: client_id.into(),
                client_secret: client_secret.into(),
                access_token: t.access_token,
                refresh_token: t.refresh_token.unwrap_or_default(),
                expires_at: now() + t.expires_in,
            }),
            Err(e) => Poll::Failed(e.to_string()),
        },
    }
}

fn refresh(agent: &ureq::Agent, client_id: &str, client_secret: &str, refresh_token: &str) -> Result<TokenResponse> {
    let v = form_post(
        agent,
        TOKEN_URL,
        &[
            ("client_id", client_id),
            ("client_secret", client_secret),
            ("refresh_token", refresh_token),
            ("grant_type", "refresh_token"),
        ],
    )?;
    if let Some(err) = v.get("error").and_then(|e| e.as_str()) {
        bail!("couldn't refresh the sign-in ({err}); sign in again");
    }
    Ok(serde_json::from_value(v)?)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sapisidhash_matches_reference() {
        // sha1("1700000000 abc/def https://music.youtube.com")
        let h = sapisid_hash("abc/def", ORIGIN, 1_700_000_000);
        assert!(h.starts_with("SAPISIDHASH 1700000000_"));
        let hex = h.rsplit('_').next().unwrap();
        assert_eq!(hex.len(), 40);
        let expected: String = Sha1::digest(b"1700000000 abc/def https://music.youtube.com")
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect();
        assert_eq!(hex, expected);
    }

    #[test]
    fn browser_input_formats() {
        let firefox = "POST /youtubei/v1/browse HTTP/2\nHost: music.youtube.com\nX-Goog-AuthUser: 1\nCookie: PREF=x; SAPISID=abc; __Secure-3PAPISID=abc\n";
        match parse_browser_input(firefox).unwrap() {
            Auth::Browser { cookie, auth_user } => {
                assert!(cookie.starts_with("PREF=x"));
                assert_eq!(auth_user, "1");
            }
            _ => panic!(),
        }
        let chrome = "cookie:\nYSC=1; SAPISID=zz\nx-goog-authuser:\n0\n";
        assert!(matches!(parse_browser_input(chrome), Ok(Auth::Browser { .. })));
        let bare = "YSC=1; SAPISID=zz; HSID=q";
        assert!(matches!(parse_browser_input(bare), Ok(Auth::Browser { .. })));
        assert!(parse_browser_input("Cookie: YSC=1").is_err());
        assert!(parse_browser_input("").is_err());
    }

    #[test]
    fn cookies_txt() {
        let txt = netscape_cookies("A=1; __Secure-3PSID=2");
        assert!(txt.contains(".youtube.com\tTRUE\t/\tFALSE\t"));
        assert!(txt.contains("\t__Secure-3PSID\t2\n"));
        assert_eq!(cookie_value("A=1; B=2=3", "B").as_deref(), Some("2=3"));
    }

    #[test]
    fn auth_round_trips() {
        let a = Auth::OAuth {
            client_id: "id".into(),
            client_secret: "s".into(),
            access_token: "t".into(),
            refresh_token: "r".into(),
            expires_at: 5,
        };
        let back: Auth = serde_json::from_str(&serde_json::to_string(&a).unwrap()).unwrap();
        assert_eq!(a, back);
    }
}
