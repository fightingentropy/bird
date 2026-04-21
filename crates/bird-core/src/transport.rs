use std::collections::BTreeMap;
use std::env;
#[cfg(target_os = "macos")]
use std::ffi::CString;
use std::sync::Mutex;
use std::time::Duration;

use anyhow::{Context, bail};
use bird_curl_impersonate_sys as impersonate_sys;
use curl::{
    Error as CurlError,
    easy::{Easy, HttpVersion, List},
};
use serde::Serialize;
use url::Url;

const DEFAULT_IMPERSONATION_PROFILE: &str = "chrome136";
const TRANSPORT_BACKEND: &str = "libcurl";
const TRANSPORT_MODE_NATIVE: &str = "native-impersonation";
const TRANSPORT_MODE_PLAIN: &str = "plain";
const TRANSPORT_PLATFORM_NATIVE: &str = "macos-native";
const TRANSPORT_PLATFORM_PLAIN: &str = "plain";
const TRANSPORT_SCOPE: &str = "twitter-hosts-only";
const SUPPORTED_IMPERSONATION_PROFILES: &[&str] = &[
    "chrome99",
    "chrome99_android",
    "chrome100",
    "chrome101",
    "chrome104",
    "chrome107",
    "chrome110",
    "chrome116",
    "chrome119",
    "chrome120",
    "chrome123",
    "chrome124",
    "chrome131",
    "chrome131_android",
    "chrome133a",
    "chrome136",
    "chrome142",
    "chrome145",
    "edge99",
    "edge101",
    "firefox133",
    "firefox135",
    "firefox144",
    "firefox147",
    "safari153",
    "safari155",
    "safari170",
    "safari172_ios",
    "safari180",
    "safari180_ios",
    "safari184",
    "safari184_ios",
    "safari260",
    "safari2601",
    "safari260_ios",
    "tor145",
];

#[derive(Debug, Clone, Default)]
pub struct HttpRequest {
    pub method: String,
    pub url: String,
    pub headers: Vec<(String, String)>,
    pub body: Option<Vec<u8>>,
    pub timeout: Option<Duration>,
}

#[derive(Debug, Clone, Default)]
pub struct HttpResponse {
    pub status: u32,
    pub headers: BTreeMap<String, String>,
    pub body: Vec<u8>,
}

impl HttpResponse {
    pub fn is_success(&self) -> bool {
        (200..300).contains(&self.status)
    }

    pub fn text(&self) -> String {
        String::from_utf8_lossy(&self.body).into_owned()
    }

    pub fn json(&self) -> anyhow::Result<serde_json::Value> {
        serde_json::from_slice(&self.body).context("failed to decode JSON body")
    }
}

pub trait HttpTransport: Send + Sync {
    fn send(&self, request: &HttpRequest) -> anyhow::Result<HttpResponse>;
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct TransportInfo {
    pub backend: &'static str,
    pub mode: &'static str,
    pub platform: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub profile: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub profile_source: Option<&'static str>,
    pub scope: &'static str,
    pub valid: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

#[derive(Debug)]
pub struct CurlTransport {
    proxy: Option<String>,
    impersonation: ImpersonationConfig,
    easy: Mutex<Easy>,
}

impl CurlTransport {
    pub fn new(proxy: Option<String>) -> Self {
        Self {
            proxy,
            impersonation: ImpersonationConfig::detect(),
            easy: Mutex::new(Easy::new()),
        }
    }

    pub fn info(&self) -> TransportInfo {
        self.impersonation.info()
    }

    fn easy_send(&self, request: &HttpRequest) -> anyhow::Result<HttpResponse> {
        let mut easy = self
            .easy
            .lock()
            .map_err(|_| anyhow::anyhow!("curl handle mutex poisoned"))?;
        // Reset options for a fresh request; libcurl preserves the connection
        // pool, DNS cache, and TLS session cache across resets.
        easy.reset();

        easy.url(&request.url)?;
        if let Some(timeout) = request.timeout {
            easy.timeout(timeout)?;
        }
        if let Some(proxy) = &self.proxy {
            easy.proxy(proxy)?;
        }

        configure_easy_browser_profile(&mut easy);
        if let Some(profile) = self.impersonation.profile_for_url(&request.url)? {
            apply_impersonation_profile(&easy, profile)?;
        }

        match request.method.to_ascii_uppercase().as_str() {
            "GET" => {
                easy.get(true)?;
            }
            "POST" => {
                easy.post(true)?;
            }
            method => {
                easy.custom_request(method)?;
            }
        }

        if let Some(body) = &request.body {
            easy.post_fields_copy(body)?;
        }

        let mut headers = List::new();
        for (key, value) in &request.headers {
            headers.append(&format!("{key}: {value}"))?;
        }
        easy.http_headers(headers)?;

        let mut response_headers = BTreeMap::new();
        let mut response_body = Vec::new();
        {
            let mut transfer = easy.transfer();
            transfer.write_function(|chunk| {
                response_body.extend_from_slice(chunk);
                Ok(chunk.len())
            })?;
            transfer.header_function(|line| {
                if let Ok(line) = std::str::from_utf8(line) {
                    if let Some((name, value)) = line.split_once(':') {
                        response_headers
                            .insert(name.trim().to_ascii_lowercase(), value.trim().to_owned());
                    }
                }
                true
            })?;
            transfer.perform()?;
        }

        Ok(HttpResponse {
            status: easy.response_code()?,
            headers: response_headers,
            body: response_body,
        })
    }
}

impl HttpTransport for CurlTransport {
    fn send(&self, request: &HttpRequest) -> anyhow::Result<HttpResponse> {
        self.easy_send(request)
    }
}

impl Default for CurlTransport {
    fn default() -> Self {
        Self::new(None)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum ImpersonationConfig {
    Disabled,
    Enabled {
        profile: String,
        source: ProfileSource,
    },
    Invalid {
        message: String,
        source: ProfileSource,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ProfileSource {
    Default,
    Environment,
}

impl ProfileSource {
    fn label(self) -> &'static str {
        match self {
            Self::Default => "default",
            Self::Environment => "BIRD_CURL_IMPERSONATE",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ResolvedImpersonationProfile {
    profile: String,
    source: ProfileSource,
}

impl ImpersonationConfig {
    fn detect() -> Self {
        if !impersonate_sys::native_impersonation_enabled() {
            return Self::Disabled;
        }

        match resolve_impersonation_profile(read_env_nonempty("BIRD_CURL_IMPERSONATE").as_deref()) {
            Ok(resolved) => Self::Enabled {
                profile: resolved.profile,
                source: resolved.source,
            },
            Err(error) => Self::Invalid {
                message: error.to_string(),
                source: ProfileSource::Environment,
            },
        }
    }

    fn info(&self) -> TransportInfo {
        match self {
            Self::Disabled => TransportInfo {
                backend: TRANSPORT_BACKEND,
                mode: TRANSPORT_MODE_PLAIN,
                platform: TRANSPORT_PLATFORM_PLAIN,
                profile: None,
                profile_source: None,
                scope: TRANSPORT_SCOPE,
                valid: true,
                error: None,
            },
            Self::Enabled { profile, source } => TransportInfo {
                backend: TRANSPORT_BACKEND,
                mode: TRANSPORT_MODE_NATIVE,
                platform: TRANSPORT_PLATFORM_NATIVE,
                profile: Some(profile.clone()),
                profile_source: Some(source.label()),
                scope: TRANSPORT_SCOPE,
                valid: true,
                error: None,
            },
            Self::Invalid { message, source } => TransportInfo {
                backend: TRANSPORT_BACKEND,
                mode: TRANSPORT_MODE_NATIVE,
                platform: TRANSPORT_PLATFORM_NATIVE,
                profile: None,
                profile_source: Some(source.label()),
                scope: TRANSPORT_SCOPE,
                valid: false,
                error: Some(message.clone()),
            },
        }
    }

    fn profile_for_url<'a>(&'a self, url: &str) -> anyhow::Result<Option<&'a str>> {
        if !is_twitter_host(url) {
            return Ok(None);
        }

        match self {
            Self::Disabled => Ok(None),
            Self::Enabled { profile, .. } => Ok(Some(profile.as_str())),
            Self::Invalid { message, .. } => bail!("{message}"),
        }
    }
}

fn resolve_impersonation_profile(
    value: Option<&str>,
) -> anyhow::Result<ResolvedImpersonationProfile> {
    let trimmed = value.map(str::trim).filter(|value| !value.is_empty());
    let (profile, source) = match trimmed {
        Some(profile) => (profile, ProfileSource::Environment),
        None => (DEFAULT_IMPERSONATION_PROFILE, ProfileSource::Default),
    };
    if SUPPORTED_IMPERSONATION_PROFILES.contains(&profile) {
        return Ok(ResolvedImpersonationProfile {
            profile: profile.to_owned(),
            source,
        });
    }

    bail!(
        "unsupported BIRD_CURL_IMPERSONATE profile `{profile}`; supported profiles: {}",
        SUPPORTED_IMPERSONATION_PROFILES.join(", ")
    )
}

#[cfg(target_os = "macos")]
fn apply_impersonation_profile(easy: &Easy, profile: &str) -> anyhow::Result<()> {
    let profile_cstr =
        CString::new(profile).context("impersonation profile contains an unexpected NUL byte")?;
    let code = unsafe { impersonate_sys::easy_impersonate(easy.raw(), profile_cstr.as_ptr(), 0) };
    if code == impersonate_sys::CURLE_OK {
        return Ok(());
    }

    Err(anyhow::Error::new(CurlError::new(code)).context(format!(
        "failed to apply BIRD_CURL_IMPERSONATE profile `{profile}`"
    )))
}

#[cfg(not(target_os = "macos"))]
fn apply_impersonation_profile(_easy: &Easy, _profile: &str) -> anyhow::Result<()> {
    Ok(())
}

fn configure_easy_browser_profile(easy: &mut Easy) {
    let _ = easy.http_version(HttpVersion::V2TLS);
    let _ = easy.accept_encoding("");
    let _ = easy.http_content_decoding(true);
}

fn is_twitter_host(url: &str) -> bool {
    let Ok(url) = Url::parse(url) else {
        return false;
    };
    matches!(
        url.host_str(),
        Some("x.com" | "api.x.com" | "twitter.com" | "api.twitter.com" | "upload.twitter.com")
    )
}

fn read_env_nonempty(name: &str) -> Option<String> {
    env::var(name)
        .ok()
        .map(|value| value.trim().to_owned())
        .filter(|value| !value.is_empty())
}

#[cfg(test)]
mod tests {
    use super::{
        DEFAULT_IMPERSONATION_PROFILE, ImpersonationConfig, ProfileSource, TransportInfo,
        is_twitter_host, resolve_impersonation_profile,
    };
    use bird_curl_impersonate_sys as impersonate_sys;

    #[test]
    fn default_impersonation_profile_is_chrome136() {
        let resolved = resolve_impersonation_profile(None).unwrap();
        assert_eq!(resolved.profile, DEFAULT_IMPERSONATION_PROFILE);
        assert_eq!(resolved.source, ProfileSource::Default);
    }

    #[test]
    fn explicit_impersonation_profile_is_respected() {
        let resolved = resolve_impersonation_profile(Some("chrome145")).unwrap();
        assert_eq!(resolved.profile, "chrome145");
        assert_eq!(resolved.source, ProfileSource::Environment);
    }

    #[test]
    fn invalid_impersonation_profile_is_rejected() {
        let error = resolve_impersonation_profile(Some("not-a-browser")).unwrap_err();
        assert!(
            error
                .to_string()
                .contains("unsupported BIRD_CURL_IMPERSONATE profile")
        );
    }

    #[test]
    fn impersonation_only_applies_to_twitter_hosts() {
        let config = ImpersonationConfig::Enabled {
            profile: "chrome136".into(),
            source: ProfileSource::Default,
        };
        assert_eq!(
            config
                .profile_for_url("https://x.com/i/api/graphql/test")
                .unwrap(),
            Some("chrome136")
        );
        assert_eq!(config.profile_for_url("https://example.com").unwrap(), None);
    }

    #[test]
    fn invalid_impersonation_config_only_errors_for_twitter_hosts() {
        let config = ImpersonationConfig::Invalid {
            message: "bad profile".into(),
            source: ProfileSource::Environment,
        };
        assert!(config.profile_for_url("https://x.com/home").is_err());
        assert_eq!(config.profile_for_url("https://example.com").unwrap(), None);
    }

    #[test]
    fn twitter_host_detection_matches_supported_hosts() {
        for url in [
            "https://x.com/home",
            "https://api.x.com/graphql/test",
            "https://twitter.com/home",
            "https://api.twitter.com/1.1/account/settings.json",
            "https://upload.twitter.com/1.1/media/upload.json",
        ] {
            assert!(is_twitter_host(url), "{url} should be matched");
        }

        for url in [
            "https://example.com",
            "https://pbs.twimg.com/media/test.jpg",
            "not-a-url",
        ] {
            assert!(!is_twitter_host(url), "{url} should not be matched");
        }
    }

    #[test]
    fn transport_info_matches_build_mode() {
        let info = if impersonate_sys::native_impersonation_enabled() {
            ImpersonationConfig::Enabled {
                profile: DEFAULT_IMPERSONATION_PROFILE.into(),
                source: ProfileSource::Default,
            }
            .info()
        } else {
            ImpersonationConfig::Disabled.info()
        };

        if impersonate_sys::native_impersonation_enabled() {
            assert_eq!(
                info,
                TransportInfo {
                    backend: "libcurl",
                    mode: "native-impersonation",
                    platform: "macos-native",
                    profile: Some("chrome136".into()),
                    profile_source: Some("default"),
                    scope: "twitter-hosts-only",
                    valid: true,
                    error: None,
                }
            );
        } else {
            assert_eq!(
                info,
                TransportInfo {
                    backend: "libcurl",
                    mode: "plain",
                    platform: "plain",
                    profile: None,
                    profile_source: None,
                    scope: "twitter-hosts-only",
                    valid: true,
                    error: None,
                }
            );
        }
    }

    #[test]
    fn invalid_transport_info_is_reported_without_network() {
        let info = ImpersonationConfig::Invalid {
            message: "bad profile".into(),
            source: ProfileSource::Environment,
        }
        .info();
        assert!(!info.valid);
        assert_eq!(info.profile, None);
        assert_eq!(info.profile_source, Some("BIRD_CURL_IMPERSONATE"));
        assert_eq!(info.error.as_deref(), Some("bad profile"));
    }
}
