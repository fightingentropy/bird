use std::fmt;
use std::path::PathBuf;
use std::time::Duration;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum BrowserName {
    Chrome,
    Edge,
    Firefox,
    Safari,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum CookieSameSite {
    Strict,
    Lax,
    None,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CookieSourceInfo {
    pub browser: BrowserName,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub profile: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub origin: Option<String>,
    #[serde(rename = "storeId", skip_serializing_if = "Option::is_none")]
    pub store_id: Option<String>,
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Cookie {
    pub name: String,
    #[serde(serialize_with = "serialize_redacted_value")]
    pub value: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub domain: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub expires: Option<i64>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub secure: bool,
    #[serde(
        default,
        rename = "httpOnly",
        skip_serializing_if = "std::ops::Not::not"
    )]
    pub http_only: bool,
    #[serde(rename = "sameSite", skip_serializing_if = "Option::is_none")]
    pub same_site: Option<CookieSameSite>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source: Option<CookieSourceInfo>,
}

impl fmt::Debug for Cookie {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("Cookie")
            .field("name", &self.name)
            .field("value", &"[REDACTED]")
            .field("domain", &self.domain)
            .field("path", &self.path)
            .field("url", &self.url)
            .field("expires", &self.expires)
            .field("secure", &self.secure)
            .field("http_only", &self.http_only)
            .field("same_site", &self.same_site)
            .field("source", &self.source)
            .finish()
    }
}

fn serialize_redacted_value<S>(_value: &str, serializer: S) -> Result<S::Ok, S::Error>
where
    S: serde::Serializer,
{
    serializer.serialize_str("[REDACTED]")
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum CookieMode {
    #[default]
    Merge,
    First,
}

#[derive(Clone, Default)]
pub struct GetCookiesOptions {
    pub url: String,
    pub origins: Vec<String>,
    pub names: Vec<String>,
    pub browsers: Vec<BrowserName>,
    pub profile: Option<String>,
    pub chrome_profile: Option<String>,
    pub edge_profile: Option<String>,
    pub firefox_profile: Option<String>,
    pub safari_cookies_file: Option<PathBuf>,
    pub include_expired: bool,
    pub timeout: Option<Duration>,
    pub debug: bool,
    pub mode: Option<CookieMode>,
    pub inline_cookies_file: Option<PathBuf>,
    pub inline_cookies_json: Option<String>,
    pub inline_cookies_base64: Option<String>,
}

impl fmt::Debug for GetCookiesOptions {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("GetCookiesOptions")
            .field("url", &self.url)
            .field("origins", &self.origins)
            .field("names", &self.names)
            .field("browsers", &self.browsers)
            .field("profile", &self.profile)
            .field("chrome_profile", &self.chrome_profile)
            .field("edge_profile", &self.edge_profile)
            .field("firefox_profile", &self.firefox_profile)
            .field("safari_cookies_file", &self.safari_cookies_file)
            .field("include_expired", &self.include_expired)
            .field("timeout", &self.timeout)
            .field("debug", &self.debug)
            .field("mode", &self.mode)
            .field("inline_cookies_file", &self.inline_cookies_file)
            .field(
                "inline_cookies_json",
                &self.inline_cookies_json.as_ref().map(|_| "[REDACTED]"),
            )
            .field(
                "inline_cookies_base64",
                &self.inline_cookies_base64.as_ref().map(|_| "[REDACTED]"),
            )
            .finish()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GetCookiesResult {
    pub cookies: Vec<Cookie>,
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum CookieHeaderSort {
    #[default]
    Name,
    None,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct CookieHeaderOptions {
    pub dedupe_by_name: bool,
    pub sort: CookieHeaderSort,
}
