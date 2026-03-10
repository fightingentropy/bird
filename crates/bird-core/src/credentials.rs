use std::cmp::Reverse;
use std::fs;
use std::path::PathBuf;
use std::time::Duration;

use anyhow::Context;
use serde::{Deserialize, Serialize};
use sweet_cookie::{
    BrowserName, Cookie, CookieHeaderOptions, CookieHeaderSort, GetCookiesOptions,
};

use crate::transport::{HttpRequest, HttpTransport};
use crate::types::{CookieSource, ResolveCredentialsOptions, ResolvedCredentials, TwitterCookies};

const TWITTER_URL: &str = "https://x.com/";
const TWITTER_ORIGINS: &[&str] = &["https://x.com/", "https://twitter.com/"];
const DEFAULT_COOKIE_TIMEOUT: Duration = Duration::from_secs(30);
const COOKIE_CACHE_TTL_MS: u64 = 24 * 60 * 60 * 1000;
const BEARER_TOKEN: &str = "AAAAAAAAAAAAAAAAAAAAANRILgAAAAAAnNwIzUejRCOuH5E6I8xnZz4puTs%3D1Zv7ttfk8LF81IUq16cHjhLTvJu4FA33AGWWjCpTnA";

#[derive(Debug, Clone, Serialize, Deserialize)]
struct CookieCacheRecord {
    auth_token: String,
    ct0: String,
    cookie_header: Option<String>,
    source: Option<String>,
    saved_at: u64,
}

pub fn default_cookie_cache_path() -> PathBuf {
    dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".cache/bird/cookies.json")
}

pub fn resolve_credentials(
    options: ResolveCredentialsOptions,
    transport: &dyn HttpTransport,
) -> anyhow::Result<ResolvedCredentials> {
    let mut warnings = Vec::new();
    let mut cookies = TwitterCookies::default();
    let cookie_timeout = options.cookie_timeout.unwrap_or(DEFAULT_COOKIE_TIMEOUT);

    if let Some(auth_token) = options.auth_token {
        cookies.auth_token = Some(auth_token);
        cookies.source = Some("CLI argument".to_owned());
    }
    if let Some(ct0) = options.ct0 {
        cookies.ct0 = Some(ct0);
        cookies.source.get_or_insert_with(|| "CLI argument".to_owned());
    }
    read_env_cookie(&mut cookies, &["AUTH_TOKEN", "TWITTER_AUTH_TOKEN"], true);
    read_env_cookie(&mut cookies, &["CT0", "TWITTER_CT0"], false);

    if cookies.auth_token.is_some() && cookies.ct0.is_some() {
        cookies.cookie_header = Some(format!(
            "auth_token={}; ct0={}",
            cookies.auth_token.as_deref().unwrap_or_default(),
            cookies.ct0.as_deref().unwrap_or_default()
        ));
        verify_cookies(&cookies, transport, default_user_agent())?;
        return Ok(ResolvedCredentials { cookies, warnings });
    }

    if let Some(cached) = load_cookie_cache() {
        let cached_cookies = TwitterCookies {
            auth_token: Some(cached.auth_token),
            ct0: Some(cached.ct0),
            cookie_header: cached.cookie_header,
            source: cached.source,
        };
        if verify_cookies(&cached_cookies, transport, default_user_agent()).is_ok() {
            return Ok(ResolvedCredentials {
                cookies: cached_cookies,
                warnings,
            });
        }
    }

    let sources = if options.cookie_source.is_empty() {
        vec![CookieSource::Safari, CookieSource::Chrome, CookieSource::Firefox]
    } else {
        options.cookie_source
    };
    for source in sources {
        let result = read_twitter_cookies_from_browser(
            source,
            options.chrome_profile.clone(),
            options.firefox_profile.clone(),
            cookie_timeout,
        )?;
        warnings.extend(result.warnings.clone());
        if result.cookies.auth_token.is_some() && result.cookies.ct0.is_some() {
            verify_cookies(&result.cookies, transport, default_user_agent())?;
            save_cookie_cache(&result.cookies)?;
            return Ok(ResolvedCredentials {
                cookies: result.cookies,
                warnings,
            });
        }
    }

    if cookies.auth_token.is_none() {
        warnings.push("Missing auth_token - provide via --auth-token, AUTH_TOKEN env var, or login to x.com in Safari/Chrome/Firefox".to_owned());
    }
    if cookies.ct0.is_none() {
        warnings.push("Missing ct0 - provide via --ct0, CT0 env var, or login to x.com in Safari/Chrome/Firefox".to_owned());
    }
    if cookies.auth_token.is_some() && cookies.ct0.is_some() {
        cookies.cookie_header = Some(format!(
            "auth_token={}; ct0={}",
            cookies.auth_token.as_deref().unwrap_or_default(),
            cookies.ct0.as_deref().unwrap_or_default()
        ));
    }

    Ok(ResolvedCredentials { cookies, warnings })
}

fn read_twitter_cookies_from_browser(
    source: CookieSource,
    chrome_profile: Option<String>,
    firefox_profile: Option<String>,
    cookie_timeout: Duration,
) -> anyhow::Result<ResolvedCredentials> {
    let browser = match source {
        CookieSource::Safari => BrowserName::Safari,
        CookieSource::Chrome => BrowserName::Chrome,
        CookieSource::Firefox => BrowserName::Firefox,
    };
    let result = sweet_cookie::get_cookies(GetCookiesOptions {
        url: TWITTER_URL.to_owned(),
        origins: TWITTER_ORIGINS.iter().map(|origin| (*origin).to_owned()).collect(),
        names: Vec::new(),
        browsers: vec![browser],
        profile: None,
        chrome_profile: chrome_profile.clone(),
        edge_profile: None,
        firefox_profile: firefox_profile.clone(),
        safari_cookies_file: None,
        include_expired: false,
        timeout: Some(cookie_timeout),
        debug: false,
        mode: Some(sweet_cookie::CookieMode::Merge),
        inline_cookies_file: None,
        inline_cookies_json: None,
        inline_cookies_base64: None,
    })?;
    let auth_token = pick_cookie_value(&result.cookies, "auth_token");
    let ct0 = pick_cookie_value(&result.cookies, "ct0");
    let mut cookies = TwitterCookies {
        auth_token,
        ct0,
        cookie_header: None,
        source: Some(match source {
            CookieSource::Safari => "Safari".to_owned(),
            CookieSource::Chrome => {
                chrome_profile
                    .clone()
                    .map(|profile| format!("Chrome profile \"{profile}\""))
                    .unwrap_or_else(|| "Chrome default profile".to_owned())
            }
            CookieSource::Firefox => firefox_profile
                .clone()
                .map(|profile| format!("Firefox profile \"{profile}\""))
                .unwrap_or_else(|| "Firefox default profile".to_owned()),
        }),
    };
    if cookies.auth_token.is_some() && cookies.ct0.is_some() {
        cookies.cookie_header = Some(build_cookie_header_from_cookies(
            &result.cookies,
            cookies.auth_token.clone(),
            cookies.ct0.clone(),
        ));
    }
    let has_credentials = cookies.auth_token.is_some() && cookies.ct0.is_some();
    let warnings = if has_credentials {
        result.warnings
    } else {
        let mut warnings = result.warnings;
        warnings.push(match source {
            CookieSource::Safari => {
                "No Twitter cookies found in Safari. Make sure you are logged into x.com in Safari."
            }
            CookieSource::Chrome => {
                "No Twitter cookies found in Chrome. Make sure you are logged into x.com in Chrome."
            }
            CookieSource::Firefox => {
                "No Twitter cookies found in Firefox. Make sure you are logged into x.com in Firefox and the profile exists."
            }
        }
        .to_owned());
        warnings
    };
    Ok(ResolvedCredentials { cookies, warnings })
}

pub fn verify_cookies(
    cookies: &TwitterCookies,
    transport: &dyn HttpTransport,
    user_agent: &str,
) -> anyhow::Result<()> {
    let auth_token = cookies.auth_token.as_deref().context("missing auth_token")?;
    let ct0 = cookies.ct0.as_deref().context("missing ct0")?;
    let cookie_header = cookies.cookie_header.clone().unwrap_or_else(|| {
        format!("auth_token={auth_token}; ct0={ct0}")
    });
    for url in [
        "https://api.x.com/1.1/account/verify_credentials.json",
        "https://x.com/i/api/1.1/account/settings.json",
    ] {
        let response = transport.send(&HttpRequest {
            method: "GET".into(),
            url: url.to_owned(),
            headers: vec![
                ("authorization".into(), format!("Bearer {BEARER_TOKEN}")),
                ("cookie".into(), cookie_header.clone()),
                ("x-csrf-token".into(), ct0.to_owned()),
                ("x-twitter-active-user".into(), "yes".into()),
                ("x-twitter-auth-type".into(), "OAuth2Session".into()),
                ("user-agent".into(), user_agent.to_owned()),
            ],
            body: None,
            timeout: Some(Duration::from_secs(15)),
        })?;
        if matches!(response.status, 401 | 403) {
            anyhow::bail!("Cookie expired or invalid (HTTP {})", response.status);
        }
        if response.is_success() {
            return Ok(());
        }
    }
    Ok(())
}

pub fn default_user_agent() -> &'static str {
    "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/131.0.0.0 Safari/537.36"
}

fn pick_cookie_value(cookies: &[Cookie], name: &str) -> Option<String> {
    let matches = cookies
        .iter()
        .filter(|cookie| cookie.name == name)
        .collect::<Vec<_>>();
    let preferred = matches
        .iter()
        .find(|cookie| cookie.domain.as_deref().unwrap_or_default().ends_with("x.com"))
        .or_else(|| {
            matches
                .iter()
                .find(|cookie| cookie.domain.as_deref().unwrap_or_default().ends_with("twitter.com"))
        })
        .or_else(|| matches.first())?;
    Some(preferred.value.clone())
}

pub fn build_cookie_header_from_cookies(
    cookies: &[Cookie],
    auth_token: Option<String>,
    ct0: Option<String>,
) -> String {
    let mut cookies = cookies
        .iter()
        .filter(|cookie| !cookie.name.is_empty())
        .cloned()
        .collect::<Vec<_>>();
    if let Some(auth_token) = auth_token {
        cookies.push(Cookie {
            name: "auth_token".into(),
            value: auth_token,
            domain: Some("x.com".into()),
            path: Some("/".into()),
            url: None,
            expires: None,
            secure: false,
            http_only: false,
            same_site: None,
            source: None,
        });
    }
    if let Some(ct0) = ct0 {
        cookies.push(Cookie {
            name: "ct0".into(),
            value: ct0,
            domain: Some("x.com".into()),
            path: Some("/".into()),
            url: None,
            expires: None,
            secure: false,
            http_only: false,
            same_site: None,
            source: None,
        });
    }
    cookies.sort_by_key(|cookie| {
        let rank = if cookie.name == "auth_token" {
            0
        } else if cookie.name == "ct0" {
            1
        } else {
            2
        };
        (
            rank,
            Reverse(cookie_domain_rank(cookie)),
            cookie.name.clone(),
        )
    });
    sweet_cookie::to_cookie_header(
        &dedupe_by_name(cookies),
        CookieHeaderOptions {
            dedupe_by_name: false,
            sort: CookieHeaderSort::None,
        },
    )
}

fn dedupe_by_name(cookies: Vec<Cookie>) -> Vec<Cookie> {
    let mut seen = std::collections::BTreeSet::new();
    let mut deduped = Vec::new();
    for cookie in cookies {
        if seen.insert(cookie.name.clone()) {
            deduped.push(cookie);
        }
    }
    deduped
}

fn cookie_domain_rank(cookie: &Cookie) -> u8 {
    match cookie.domain.as_deref().unwrap_or_default() {
        domain if domain.ends_with("x.com") => 2,
        domain if domain.ends_with("twitter.com") => 1,
        _ => 0,
    }
}

fn read_env_cookie(cookies: &mut TwitterCookies, keys: &[&str], auth: bool) {
    let current = if auth {
        cookies.auth_token.as_ref()
    } else {
        cookies.ct0.as_ref()
    };
    if current.is_some() {
        return;
    }
    for key in keys {
        let value = std::env::var(key).ok().map(|value| value.trim().to_owned());
        let value = value.filter(|value| !value.is_empty());
        if let Some(value) = value {
            if auth {
                cookies.auth_token = Some(value);
            } else {
                cookies.ct0 = Some(value);
            }
            cookies.source.get_or_insert_with(|| format!("env {key}"));
            break;
        }
    }
}

fn load_cookie_cache() -> Option<CookieCacheRecord> {
    let path = default_cookie_cache_path();
    let raw = fs::read_to_string(path).ok()?;
    let parsed = serde_json::from_str::<CookieCacheRecord>(&raw).ok()?;
    let now = current_time_millis();
    if now.saturating_sub(parsed.saved_at) > COOKIE_CACHE_TTL_MS {
        return None;
    }
    Some(parsed)
}

fn save_cookie_cache(cookies: &TwitterCookies) -> anyhow::Result<()> {
    let Some(auth_token) = cookies.auth_token.clone() else {
        return Ok(());
    };
    let Some(ct0) = cookies.ct0.clone() else {
        return Ok(());
    };
    let path = default_cookie_cache_path();
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let payload = CookieCacheRecord {
        auth_token,
        ct0,
        cookie_header: cookies.cookie_header.clone(),
        source: cookies.source.clone(),
        saved_at: current_time_millis(),
    };
    fs::write(path, format!("{}\n", serde_json::to_string_pretty(&payload)?))?;
    Ok(())
}

fn current_time_millis() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_millis() as u64)
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use sweet_cookie::Cookie;

    use super::build_cookie_header_from_cookies;

    #[test]
    fn cookie_header_prefers_auth_and_ct0_first() {
        let header = build_cookie_header_from_cookies(
            &[
                Cookie {
                    name: "kdt".into(),
                    value: "one".into(),
                    domain: Some("twitter.com".into()),
                    path: Some("/".into()),
                    url: None,
                    expires: None,
                    secure: false,
                    http_only: false,
                    same_site: None,
                    source: None,
                },
                Cookie {
                    name: "lang".into(),
                    value: "en".into(),
                    domain: Some("x.com".into()),
                    path: Some("/".into()),
                    url: None,
                    expires: None,
                    secure: false,
                    http_only: false,
                    same_site: None,
                    source: None,
                },
            ],
            Some("auth".into()),
            Some("csrf".into()),
        );

        assert_eq!(header, "auth_token=auth; ct0=csrf; lang=en; kdt=one");
    }

    #[test]
    fn cookie_header_prefers_x_domain_for_duplicate_names() {
        let header = build_cookie_header_from_cookies(
            &[
                Cookie {
                    name: "lang".into(),
                    value: "legacy".into(),
                    domain: Some("twitter.com".into()),
                    path: Some("/".into()),
                    url: None,
                    expires: None,
                    secure: false,
                    http_only: false,
                    same_site: None,
                    source: None,
                },
                Cookie {
                    name: "lang".into(),
                    value: "modern".into(),
                    domain: Some("x.com".into()),
                    path: Some("/".into()),
                    url: None,
                    expires: None,
                    secure: false,
                    http_only: false,
                    same_site: None,
                    source: None,
                },
            ],
            None,
            None,
        );

        assert_eq!(header, "lang=modern");
    }
}
