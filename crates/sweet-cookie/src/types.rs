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

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Cookie {
    pub name: String,
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
    #[serde(default, rename = "httpOnly", skip_serializing_if = "std::ops::Not::not")]
    pub http_only: bool,
    #[serde(rename = "sameSite", skip_serializing_if = "Option::is_none")]
    pub same_site: Option<CookieSameSite>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source: Option<CookieSourceInfo>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum CookieMode {
    #[default]
    Merge,
    First,
}

#[derive(Debug, Clone, Default)]
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
