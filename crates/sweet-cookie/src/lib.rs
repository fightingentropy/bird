mod providers;
mod types;
mod util;

pub use types::{
    BrowserName, Cookie, CookieHeaderOptions, CookieHeaderSort, CookieMode, CookieSameSite,
    CookieSourceInfo, GetCookiesOptions, GetCookiesResult,
};

use std::collections::HashMap;
use std::path::PathBuf;

use anyhow::Context;
use providers::{get_cookies_from_chromium, get_cookies_from_firefox, get_cookies_from_inline, get_cookies_from_safari, InlineSource};
use url::Url;
use util::{
    default_browsers, normalize_names, normalize_origins, parse_browsers_env, parse_mode_env,
    read_env_nonempty,
};

pub fn get_cookies(options: GetCookiesOptions) -> anyhow::Result<GetCookiesResult> {
    let origins = normalize_origins(&options.url, &options.origins)?;
    let allowlist_names = normalize_names(&options.names);
    let browsers = if options.browsers.is_empty() {
        parse_browsers_env().unwrap_or_else(default_browsers)
    } else {
        options.browsers.clone()
    };
    let mode = options.mode.unwrap_or_else(|| parse_mode_env().unwrap_or_default());
    let mut warnings = Vec::new();

    for source in resolve_inline_sources(&options) {
        let mut result = get_cookies_from_inline(&source, &origins, allowlist_names.as_ref())?;
        if !result.cookies.is_empty() {
            result.warnings.splice(0..0, warnings);
            return Ok(result);
        }
        warnings.extend(result.warnings);
    }

    let mut merged = HashMap::<String, Cookie>::new();
    for browser in browsers {
        let result = match browser {
            BrowserName::Chrome => get_cookies_from_chromium(
                BrowserName::Chrome,
                options.chrome_profile.clone().or_else(|| options.profile.clone()).or_else(|| {
                    read_env_nonempty("SWEET_COOKIE_CHROME_PROFILE")
                }),
                &origins,
                allowlist_names.as_ref(),
                options.include_expired,
                options.timeout,
            )?,
            BrowserName::Edge => get_cookies_from_chromium(
                BrowserName::Edge,
                options
                    .edge_profile
                    .clone()
                    .or_else(|| options.profile.clone())
                    .or_else(|| read_env_nonempty("SWEET_COOKIE_EDGE_PROFILE"))
                    .or_else(|| read_env_nonempty("SWEET_COOKIE_CHROME_PROFILE")),
                &origins,
                allowlist_names.as_ref(),
                options.include_expired,
                options.timeout,
            )?,
            BrowserName::Firefox => get_cookies_from_firefox(
                options
                    .firefox_profile
                    .clone()
                    .or_else(|| read_env_nonempty("SWEET_COOKIE_FIREFOX_PROFILE")),
                &origins,
                allowlist_names.as_ref(),
                options.include_expired,
            )?,
            BrowserName::Safari => get_cookies_from_safari(
                options.safari_cookies_file.clone(),
                &origins,
                allowlist_names.as_ref(),
                options.include_expired,
            )?,
        };
        warnings.extend(result.warnings.clone());
        if matches!(mode, CookieMode::First) && !result.cookies.is_empty() {
            return Ok(GetCookiesResult {
                cookies: result.cookies,
                warnings,
            });
        }
        for cookie in result.cookies {
            let key = format!(
                "{}|{}|{}",
                cookie.name,
                cookie.domain.as_deref().unwrap_or_default(),
                cookie.path.as_deref().unwrap_or_default()
            );
            merged.entry(key).or_insert(cookie);
        }
    }

    Ok(GetCookiesResult {
        cookies: merged.into_values().collect(),
        warnings,
    })
}

pub fn to_cookie_header(cookies: &[Cookie], options: CookieHeaderOptions) -> String {
    let mut items = cookies
        .iter()
        .filter(|cookie| !cookie.name.is_empty())
        .map(|cookie| (cookie.name.clone(), cookie.value.clone()))
        .collect::<Vec<_>>();
    if matches!(options.sort, CookieHeaderSort::Name) {
        items.sort_by(|left, right| left.0.cmp(&right.0));
    }
    if !options.dedupe_by_name {
        return items
            .into_iter()
            .map(|(name, value)| format!("{name}={value}"))
            .collect::<Vec<_>>()
            .join("; ");
    }

    let mut seen = std::collections::HashSet::new();
    items
        .into_iter()
        .filter(|(name, _)| seen.insert(name.clone()))
        .map(|(name, value)| format!("{name}={value}"))
        .collect::<Vec<_>>()
        .join("; ")
}

fn resolve_inline_sources(options: &GetCookiesOptions) -> Vec<InlineSource> {
    let mut sources = Vec::new();
    if let Some(payload) = &options.inline_cookies_json {
        sources.push(InlineSource {
            source: "inline-json".to_owned(),
            payload: payload.clone(),
        });
    }
    if let Some(payload) = &options.inline_cookies_base64 {
        sources.push(InlineSource {
            source: "inline-base64".to_owned(),
            payload: payload.clone(),
        });
    }
    if let Some(path) = &options.inline_cookies_file {
        sources.push(InlineSource {
            source: "inline-file".to_owned(),
            payload: path.to_string_lossy().into_owned(),
        });
    }
    sources
}

pub fn browsers_for_cli(values: &[String]) -> anyhow::Result<Vec<BrowserName>> {
    values
        .iter()
        .map(|value| match value.trim().to_ascii_lowercase().as_str() {
            "chrome" => Ok(BrowserName::Chrome),
            "edge" => Ok(BrowserName::Edge),
            "firefox" => Ok(BrowserName::Firefox),
            "safari" => Ok(BrowserName::Safari),
            other => anyhow::bail!("unsupported browser {other}"),
        })
        .collect()
}

pub fn parse_mode(value: Option<&str>) -> anyhow::Result<Option<CookieMode>> {
    match value.map(str::trim).filter(|value| !value.is_empty()) {
        Some("merge") => Ok(Some(CookieMode::Merge)),
        Some("first") => Ok(Some(CookieMode::First)),
        Some(other) => anyhow::bail!("unsupported mode {other}"),
        None => Ok(None),
    }
}

pub fn parse_url(value: &str) -> anyhow::Result<Url> {
    Url::parse(value).with_context(|| format!("invalid URL {value}"))
}

pub fn parse_path(value: Option<&str>) -> Option<PathBuf> {
    value.map(PathBuf::from)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::util::dedupe_cookies;

    #[test]
    fn cookie_header_sorts_and_dedupes() {
        let cookies = vec![
            Cookie {
                name: "ct0".to_owned(),
                value: "123".to_owned(),
                domain: Some("x.com".to_owned()),
                path: Some("/".to_owned()),
                url: None,
                expires: None,
                secure: false,
                http_only: false,
                same_site: None,
                source: None,
            },
            Cookie {
                name: "auth_token".to_owned(),
                value: "abc".to_owned(),
                domain: Some("x.com".to_owned()),
                path: Some("/".to_owned()),
                url: None,
                expires: None,
                secure: false,
                http_only: false,
                same_site: None,
                source: None,
            },
            Cookie {
                name: "auth_token".to_owned(),
                value: "override".to_owned(),
                domain: Some("twitter.com".to_owned()),
                path: Some("/".to_owned()),
                url: None,
                expires: None,
                secure: false,
                http_only: false,
                same_site: None,
                source: None,
            },
        ];
        let header = to_cookie_header(
            &cookies,
            CookieHeaderOptions {
                dedupe_by_name: true,
                sort: CookieHeaderSort::Name,
            },
        );
        assert_eq!(header, "auth_token=abc; ct0=123");
    }

    #[test]
    fn mode_parser_accepts_known_values() {
        assert_eq!(parse_mode(Some("merge")).unwrap(), Some(CookieMode::Merge));
        assert_eq!(parse_mode(Some("first")).unwrap(), Some(CookieMode::First));
        assert!(parse_mode(Some("unknown")).is_err());
    }

    #[test]
    fn dedupe_keeps_first_cookie() {
        let deduped = dedupe_cookies(vec![
            Cookie {
                name: "auth_token".to_owned(),
                value: "abc".to_owned(),
                domain: Some("x.com".to_owned()),
                path: Some("/".to_owned()),
                url: None,
                expires: None,
                secure: false,
                http_only: false,
                same_site: None,
                source: None,
            },
            Cookie {
                name: "auth_token".to_owned(),
                value: "def".to_owned(),
                domain: Some("x.com".to_owned()),
                path: Some("/".to_owned()),
                url: None,
                expires: None,
                secure: false,
                http_only: false,
                same_site: None,
                source: None,
            },
        ]);
        assert_eq!(deduped.len(), 1);
        assert_eq!(deduped[0].value, "abc");
    }
}
