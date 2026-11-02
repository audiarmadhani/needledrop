//! Signing in with the cookies of a browser session that is logged in to
//! YouTube, the same approach as ytmusicapi's "browser" auth and yt-dlp.
//!
//! The browser keeps rotating some of these cookies while you use YouTube,
//! so a saved copy goes stale. `login` remembers which browser and account
//! you chose, and each launch re-reads fresh cookies from that browser,
//! falling back to the saved copy (a Netscape `cookies.txt`).

use std::fs::{self, OpenOptions};
use std::io::Write;
use std::os::unix::fs::OpenOptionsExt;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result, anyhow, bail};
use clap::ValueEnum;
use sha1::{Digest, Sha1};

pub const ORIGIN: &str = "https://music.youtube.com";

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum Browser {
    Chrome,
    Arc,
    Brave,
    Edge,
    Chromium,
    Vivaldi,
    Opera,
    Firefox,
    Zen,
    Librewolf,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Cookie {
    pub domain: String,
    pub path: String,
    pub secure: bool,
    /// Unix seconds; 0 means a session cookie.
    pub expires: u64,
    pub name: String,
    pub value: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Credentials {
    cookies: Vec<Cookie>,
    /// Which of the browser's signed-in Google accounts to use
    /// (the `X-Goog-AuthUser` index, 0 = the browser's default account).
    pub account: u32,
    /// Where the cookies came from, to refresh them on launch.
    /// `None` for a pasted `Cookie` header.
    pub browser: Option<Browser>,
}

impl Credentials {
    /// `$XDG_CONFIG_HOME/needledrop/cookies.txt`, else `~/.config/needledrop/cookies.txt`.
    fn dir() -> Result<PathBuf> {
        let config = match std::env::var_os("XDG_CONFIG_HOME") {
            Some(dir) if !dir.is_empty() => PathBuf::from(dir),
            _ => PathBuf::from(std::env::var_os("HOME").context("HOME is not set")?).join(".config"),
        };
        Ok(config.join("needledrop"))
    }

    /// The saved cookies, in Netscape format so yt-dlp can read them too.
    pub fn path() -> Result<PathBuf> {
        Ok(Self::dir()?.join("cookies.txt"))
    }

    /// `browser=<name>` and `account=<index>` lines.
    fn login_path() -> Result<PathBuf> {
        Ok(Self::dir()?.join("login"))
    }

    pub fn load() -> Result<Option<Self>> {
        let path = Self::path()?;
        let text = match fs::read_to_string(&path) {
            Ok(text) => text,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(error).with_context(|| format!("could not read {}", path.display())),
        };
        let mut credentials = Self::from_netscape(&text);
        let login = fs::read_to_string(Self::login_path()?).unwrap_or_default();
        for (key, value) in login.lines().filter_map(|line| line.split_once('=')) {
            match key {
                "account" => credentials.account = value.parse().unwrap_or(0),
                "browser" => credentials.browser = Browser::from_str(value, true).ok(),
                _ => {}
            }
        }
        Ok(Some(credentials))
    }

    /// Re-reads the cookies from the browser they came from and saves them.
    /// Keeps the current cookies if that fails (browser closed, Keychain denied…).
    pub fn refreshed(self) -> Self {
        let Some(browser) = self.browser else { return self };
        match Self::from_browser(browser) {
            Ok(fresh) => {
                let fresh = Self {
                    account: self.account,
                    ..fresh
                };
                let _ = fresh.save();
                fresh
            }
            Err(_) => self,
        }
    }

    /// Writes the cookies and account choice, readable by the owner only.
    pub fn save(&self) -> Result<PathBuf> {
        let path = Self::path()?;
        write_private(&path, &self.to_netscape())?;
        let mut login = format!("account={}\n", self.account);
        if let Some(browser) = self.browser.and_then(|b| b.to_possible_value()) {
            login.push_str(&format!("browser={}\n", browser.get_name()));
        }
        write_private(&Self::login_path()?, &login)?;
        Ok(path)
    }

    /// Deletes the saved login. Returns whether there was one.
    pub fn delete() -> Result<bool> {
        let _ = fs::remove_file(Self::login_path()?);
        match fs::remove_file(Self::path()?) {
            Ok(()) => Ok(true),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
            Err(error) => Err(error.into()),
        }
    }

    pub fn with_account(&self, account: u32) -> Self {
        Self {
            account,
            ..self.clone()
        }
    }

    pub fn from_browser(browser: Browser) -> Result<Self> {
        let domains = Some(vec!["youtube.com".to_owned()]);
        let cookies = match browser {
            Browser::Chrome => rookie::chrome(domains),
            Browser::Arc => rookie::arc(domains),
            Browser::Brave => rookie::brave(domains),
            Browser::Edge => rookie::edge(domains),
            Browser::Chromium => rookie::chromium(domains),
            Browser::Vivaldi => rookie::vivaldi(domains),
            Browser::Opera => rookie::opera(domains),
            Browser::Firefox => rookie::firefox(domains),
            Browser::Zen => rookie::zen(domains),
            Browser::Librewolf => rookie::librewolf(domains),
        }
        .map_err(|error| anyhow!("could not read cookies from {browser:?}: {error}"))?;

        let cookies = cookies
            .into_iter()
            .filter(|c| is_youtube_domain(&c.domain))
            .map(|c| Cookie {
                domain: c.domain,
                path: c.path,
                secure: c.secure,
                expires: c.expires.unwrap_or(0),
                name: c.name,
                value: c.value,
            })
            .collect();
        Self::validated(Self {
            cookies,
            account: 0,
            browser: Some(browser),
        })
        .with_context(|| format!("sign in to music.youtube.com in {browser:?} first"))
    }

    /// Builds credentials from a `Cookie:` request header copied from devtools.
    pub fn from_cookie_header(header: &str) -> Result<Self> {
        let header = header.trim();
        let header = header
            .strip_prefix("Cookie:")
            .or_else(|| header.strip_prefix("cookie:"))
            .unwrap_or(header);
        let cookies = header
            .split(';')
            .filter_map(|pair| pair.trim().split_once('='))
            .map(|(name, value)| Cookie {
                domain: ".youtube.com".to_owned(),
                path: "/".to_owned(),
                secure: true,
                expires: 0,
                name: name.trim().to_owned(),
                value: value.trim().to_owned(),
            })
            .collect();
        Self::validated(Self {
            cookies,
            account: 0,
            browser: None,
        })
    }

    fn validated(self) -> Result<Self> {
        if self.sapisid().is_none() {
            bail!("no YouTube login cookie (SAPISID) found");
        }
        Ok(self)
    }

    fn sapisid(&self) -> Option<&str> {
        ["SAPISID", "__Secure-3PAPISID"]
            .iter()
            .find_map(|name| self.cookies.iter().find(|c| c.name == *name))
            .map(|c| c.value.as_str())
    }

    /// The `Cookie` header for requests to music.youtube.com.
    pub fn cookie_header(&self) -> String {
        let now = unix_now();
        let mut seen = std::collections::HashSet::new();
        self.cookies
            .iter()
            .filter(|c| c.expires == 0 || c.expires > now)
            .filter(|c| seen.insert(c.name.as_str()))
            .map(|c| format!("{}={}", c.name, c.value))
            .collect::<Vec<_>>()
            .join("; ")
    }

    /// The `Authorization` header YouTube expects alongside the cookies:
    /// `SAPISIDHASH <ts>_<sha1("<ts> <SAPISID> <origin>")>`.
    pub fn authorization(&self) -> String {
        sapisid_hash(self.sapisid().unwrap_or_default(), unix_now())
    }

    fn to_netscape(&self) -> String {
        let mut text =
            String::from("# Netscape HTTP Cookie File\n# Written by needledrop. Keep this file private.\n\n");
        for c in &self.cookies {
            let flag = if c.domain.starts_with('.') { "TRUE" } else { "FALSE" };
            let secure = if c.secure { "TRUE" } else { "FALSE" };
            text.push_str(&format!(
                "{}\t{flag}\t{}\t{secure}\t{}\t{}\t{}\n",
                c.domain, c.path, c.expires, c.name, c.value
            ));
        }
        text
    }

    fn from_netscape(text: &str) -> Self {
        let cookies = text
            .lines()
            .map(|line| line.strip_prefix("#HttpOnly_").unwrap_or(line))
            .filter(|line| !line.trim().is_empty() && !line.starts_with('#'))
            .filter_map(|line| {
                let fields: Vec<&str> = line.split('\t').collect();
                let [domain, _, path, secure, expires, name, value] = fields[..] else {
                    return None;
                };
                Some(Cookie {
                    domain: domain.to_owned(),
                    path: path.to_owned(),
                    secure: secure == "TRUE",
                    expires: expires.parse().unwrap_or(0),
                    name: name.to_owned(),
                    value: value.to_owned(),
                })
            })
            .collect();
        Self {
            cookies,
            account: 0,
            browser: None,
        }
    }
}

fn write_private(path: &std::path::Path, contents: &str) -> Result<()> {
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir).with_context(|| format!("could not create {}", dir.display()))?;
    }
    let mut file = OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(0o600)
        .open(path)
        .with_context(|| format!("could not write {}", path.display()))?;
    file.write_all(contents.as_bytes())?;
    Ok(())
}

fn is_youtube_domain(domain: &str) -> bool {
    let domain = domain.trim_start_matches('.');
    domain == "youtube.com" || domain.ends_with(".youtube.com")
}

fn sapisid_hash(sapisid: &str, timestamp: u64) -> String {
    let digest = Sha1::digest(format!("{timestamp} {sapisid} {ORIGIN}"));
    let hex: String = digest.iter().map(|b| format!("{b:02x}")).collect();
    format!("SAPISIDHASH {timestamp}_{hex}")
}

fn unix_now() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sapisid_hash_matches_reference() {
        // Reference: printf '1700000000 abc/def https://music.youtube.com' | shasum
        assert_eq!(
            sapisid_hash("abc/def", 1_700_000_000),
            "SAPISIDHASH 1700000000_523b01f033dd8ed738175261f406736fe239c73d"
        );
    }

    #[test]
    fn parses_pasted_cookie_header() {
        let creds = Credentials::from_cookie_header("Cookie: SID=s1; HSID=h; SAPISID=sap ; PREF=a=b").unwrap();
        assert_eq!(creds.sapisid(), Some("sap"));
        assert_eq!(creds.cookie_header(), "SID=s1; HSID=h; SAPISID=sap; PREF=a=b");
    }

    #[test]
    fn rejects_cookies_without_login() {
        assert!(Credentials::from_cookie_header("PREF=x; VISITOR_INFO1_LIVE=y").is_err());
    }

    #[test]
    fn netscape_round_trip() {
        let creds = Credentials::from_cookie_header("SAPISID=sap; SID=s").unwrap();
        assert_eq!(Credentials::from_netscape(&creds.to_netscape()), creds);
    }

    #[test]
    fn reads_http_only_lines_and_skips_comments() {
        let creds = Credentials::from_netscape(
            "# Netscape HTTP Cookie File\n#HttpOnly_.youtube.com\tTRUE\t/\tTRUE\t0\tSID\tx\n.youtube.com\tTRUE\t/\tTRUE\t0\tSAPISID\ty\n",
        );
        assert_eq!(creds.cookie_header(), "SID=x; SAPISID=y");
    }

    #[test]
    fn skips_expired_and_duplicate_cookies() {
        let cookie = |name: &str, expires| Cookie {
            domain: ".youtube.com".into(),
            path: "/".into(),
            secure: true,
            expires,
            name: name.into(),
            value: "v".into(),
        };
        let creds = Credentials {
            account: 0,
            browser: None,
            cookies: vec![cookie("OLD", 1), cookie("SID", 0), cookie("SID", 0)],
        };
        assert_eq!(creds.cookie_header(), "SID=v");
    }
}
