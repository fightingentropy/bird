use std::collections::HashSet;
use std::env;
use std::fs;
use std::path::{Path, PathBuf};

use crate::{Cookie, CookieMode};
use url::Url;

pub(crate) fn normalize_origins(url: &str, extra_origins: &[String]) -> anyhow::Result<Vec<Url>> {
    let mut origins = Vec::new();
    origins.push(ensure_origin_url(Url::parse(url)?)?);
    for origin in extra_origins {
        if origin.trim().is_empty() {
            continue;
        }
        if let Ok(parsed) = Url::parse(origin) {
            origins.push(ensure_origin_url(parsed)?);
        }
    }
    origins.sort_by(|left, right| left.as_str().cmp(right.as_str()));
    origins.dedup_by(|left, right| left == right);
    Ok(origins)
}

fn ensure_origin_url(mut url: Url) -> anyhow::Result<Url> {
    if url.origin().ascii_serialization() == "null" {
        anyhow::bail!("origin URL must include a scheme and hostname");
    }
    url.set_path("/");
    url.set_query(None);
    url.set_fragment(None);
    Ok(url)
}

pub(crate) fn normalize_names(names: &[String]) -> Option<HashSet<String>> {
    let names = names
        .iter()
        .map(|name| name.trim())
        .filter(|name| !name.is_empty())
        .map(ToOwned::to_owned)
        .collect::<HashSet<_>>();
    if names.is_empty() {
        None
    } else {
        Some(names)
    }
}

pub(crate) fn default_browsers() -> Vec<crate::BrowserName> {
    vec![
        crate::BrowserName::Chrome,
        crate::BrowserName::Safari,
        crate::BrowserName::Firefox,
    ]
}

pub(crate) fn parse_browsers_env() -> Option<Vec<crate::BrowserName>> {
    let raw = read_env_nonempty("SWEET_COOKIE_BROWSERS")
        .or_else(|| read_env_nonempty("SWEET_COOKIE_SOURCES"))?;
    let mut browsers = Vec::new();
    for token in raw
        .split([',', ' ', '\n', '\t'])
        .map(str::trim)
        .filter(|token| !token.is_empty())
    {
        let browser = match token {
            "chrome" => crate::BrowserName::Chrome,
            "edge" => crate::BrowserName::Edge,
            "firefox" => crate::BrowserName::Firefox,
            "safari" => crate::BrowserName::Safari,
            _ => continue,
        };
        if !browsers.contains(&browser) {
            browsers.push(browser);
        }
    }
    if browsers.is_empty() {
        None
    } else {
        Some(browsers)
    }
}

pub(crate) fn parse_mode_env() -> Option<CookieMode> {
    match read_env_nonempty("SWEET_COOKIE_MODE")?.to_lowercase().as_str() {
        "merge" => Some(CookieMode::Merge),
        "first" => Some(CookieMode::First),
        _ => None,
    }
}

pub(crate) fn read_env_nonempty(key: &str) -> Option<String> {
    env::var(key).ok().map(|value| value.trim().to_owned()).filter(|value| !value.is_empty())
}

pub(crate) fn dedupe_cookies(cookies: impl IntoIterator<Item = Cookie>) -> Vec<Cookie> {
    let mut seen = HashSet::new();
    let mut merged = Vec::new();
    for cookie in cookies {
        let key = format!(
            "{}|{}|{}",
            cookie.name,
            cookie.domain.as_deref().unwrap_or_default(),
            cookie.path.as_deref().unwrap_or_default()
        );
        if seen.insert(key) {
            merged.push(cookie);
        }
    }
    merged
}

pub(crate) fn host_matches_cookie_domain(host: &str, cookie_domain: &str) -> bool {
    let normalized_host = host.to_ascii_lowercase();
    let normalized_domain = cookie_domain.trim_start_matches('.').to_ascii_lowercase();
    normalized_host == normalized_domain || normalized_host.ends_with(&format!(".{normalized_domain}"))
}

pub(crate) fn cookie_matches_hosts(cookie_domain: &str, hosts: &[String]) -> bool {
    let domain = cookie_domain.trim_start_matches('.');
    hosts.iter().any(|host| host_matches_cookie_domain(host, domain))
}

pub(crate) fn normalize_expiration(expires: Option<i64>) -> Option<i64> {
    let value = expires?;
    if value <= 0 {
        return None;
    }
    if value > 10_000_000_000_000 {
        return Some((value / 1_000_000) - 11_644_473_600);
    }
    if value > 10_000_000_000 {
        return Some(value / 1000);
    }
    Some(value)
}

pub(crate) fn hosts_from_origins(origins: &[Url]) -> Vec<String> {
    origins
        .iter()
        .filter_map(|origin| origin.host_str().map(ToOwned::to_owned))
        .collect()
}

pub(crate) fn safe_hostname_from_url(raw: &str) -> Option<String> {
    Url::parse(raw)
        .ok()
        .and_then(|url| url.host_str().map(|host| host.trim_start_matches('.').to_owned()))
        .or_else(|| {
            let trimmed = raw.trim();
            if trimmed.is_empty() {
                None
            } else {
                Some(trimmed.trim_start_matches('.').to_owned())
            }
        })
}

pub(crate) fn looks_like_path(value: &str) -> bool {
    value.contains('/') || value.contains('\\')
}

pub(crate) fn expand_path(value: &str) -> PathBuf {
    if let Some(stripped) = value.strip_prefix("~/") {
        if let Some(home) = dirs::home_dir() {
            return home.join(stripped);
        }
    }
    let path = PathBuf::from(value);
    if path.is_absolute() {
        path
    } else {
        std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")).join(path)
    }
}

pub(crate) fn copy_sidecar(source_db_path: &Path, target_db_path: &Path, suffix: &str) -> std::io::Result<()> {
    let sidecar = PathBuf::from(format!("{}{}", source_db_path.display(), suffix));
    if sidecar.exists() {
        let target = PathBuf::from(format!("{}{}", target_db_path.display(), suffix));
        fs::copy(sidecar, target)?;
    }
    Ok(())
}
