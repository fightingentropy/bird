use std::collections::BTreeMap;
use std::env;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::Duration;

use anyhow::Context;
use curl::easy::{Easy, HttpVersion, List};
use tempfile::NamedTempFile;
use url::Url;

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

#[derive(Debug, Clone)]
pub struct CurlTransport {
    proxy: Option<String>,
    external: Option<ExternalCurlTransport>,
}

impl CurlTransport {
    pub fn new(proxy: Option<String>) -> Self {
        Self {
            proxy,
            external: ExternalCurlTransport::detect(),
        }
    }

    fn easy_send(&self, request: &HttpRequest) -> anyhow::Result<HttpResponse> {
        let mut easy = Easy::new();
        easy.url(&request.url)?;
        if let Some(timeout) = request.timeout {
            easy.timeout(timeout)?;
        }
        if let Some(proxy) = &self.proxy {
            easy.proxy(proxy)?;
        }

        configure_easy_browser_profile(&mut easy);

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
                        response_headers.insert(
                            name.trim().to_ascii_lowercase(),
                            value.trim().to_owned(),
                        );
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
        if let Some(external) = &self.external {
            if external.should_handle(&request.url) {
                if let Ok(response) = external.send(request, self.proxy.as_deref()) {
                    return Ok(response);
                }
            }
        }
        self.easy_send(request)
    }
}

impl Default for CurlTransport {
    fn default() -> Self {
        Self::new(None)
    }
}

#[derive(Debug, Clone)]
struct ExternalCurlTransport {
    binary: PathBuf,
    impersonate: Option<String>,
    supports_impersonate: bool,
    kind: ExternalCurlKind,
}

impl ExternalCurlTransport {
    fn detect() -> Option<Self> {
        let impersonate = read_env_nonempty("BIRD_CURL_IMPERSONATE");
        if let Some(binary) = read_env_nonempty("BIRD_CURL_BIN").map(PathBuf::from) {
            let supports_impersonate = binary_supports_option(&binary, "--impersonate");
            return Some(Self {
                kind: detect_external_kind(&binary),
                binary,
                impersonate,
                supports_impersonate,
            });
        }

        for candidate in [
            "curl_chrome136",
            "curl_chrome124",
            "curl_chrome116",
            "curl-impersonate-chrome",
            "curl-impersonate",
        ] {
            if let Some(binary) = find_in_path(candidate) {
                let supports_impersonate = binary_supports_option(&binary, "--impersonate");
                return Some(Self {
                    kind: detect_external_kind(&binary),
                    binary,
                    impersonate,
                    supports_impersonate,
                });
            }
        }

        None
    }

    fn should_handle(&self, url: &str) -> bool {
        is_twitter_host(url)
    }

    fn send(
        &self,
        request: &HttpRequest,
        proxy: Option<&str>,
    ) -> anyhow::Result<HttpResponse> {
        let header_file = NamedTempFile::new().context("failed to create curl header temp file")?;
        let mut command = Command::new(&self.binary);
        command
            .arg("--silent")
            .arg("--show-error")
            .arg("--globoff")
            .arg("--request")
            .arg(request.method.to_ascii_uppercase())
            .arg("--url")
            .arg(&request.url)
            .arg("--dump-header")
            .arg(header_file.path())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());

        self.apply_transport_profile(&mut command);

        if let Some(timeout) = request.timeout {
            command.arg("--max-time").arg(format!("{:.3}", timeout.as_secs_f64()));
        }
        if let Some(proxy) = proxy {
            command.arg("--proxy").arg(proxy);
        }
        if self.supports_impersonate {
            if let Some(impersonate) = &self.impersonate {
                command.arg("--impersonate").arg(impersonate);
            }
        }
        for (name, value) in &request.headers {
            command.arg("--header").arg(format!("{name}: {value}"));
        }
        if request.body.is_some() {
            command.arg("--data-binary").arg("@-");
            command.stdin(Stdio::piped());
        }

        let mut child = command.spawn().with_context(|| {
            format!(
                "failed to spawn external curl transport {}",
                self.binary.display()
            )
        })?;
        if let Some(body) = &request.body {
            let mut stdin = child.stdin.take().context("missing curl stdin pipe")?;
            stdin.write_all(body)?;
        }
        let output = child.wait_with_output()?;
        if !output.status.success() {
            anyhow::bail!(
                "external curl transport failed: {}",
                String::from_utf8_lossy(&output.stderr).trim()
            );
        }

        let raw_headers = std::fs::read(header_file.path())?;
        let (status, headers) = parse_curl_headers(&raw_headers)
            .context("failed to parse external curl response headers")?;

        Ok(HttpResponse {
            status,
            headers,
            body: output.stdout,
        })
    }

    fn apply_transport_profile(&self, command: &mut Command) {
        match self.kind {
            ExternalCurlKind::ChromeImpersonate => {
                command
                    .arg("--http2")
                    .arg("--http2-no-server-push")
                    .arg("--false-start")
                    .arg("--compressed")
                    .arg("--tlsv1.2")
                    .arg("--alps")
                    .arg("--tls-permute-extensions")
                    .arg("--cert-compression")
                    .arg("brotli")
                    .arg("--ciphers")
                    .arg("TLS_AES_128_GCM_SHA256,TLS_AES_256_GCM_SHA384,TLS_CHACHA20_POLY1305_SHA256,ECDHE-ECDSA-AES128-GCM-SHA256,ECDHE-RSA-AES128-GCM-SHA256,ECDHE-ECDSA-AES256-GCM-SHA384,ECDHE-RSA-AES256-GCM-SHA384,ECDHE-ECDSA-CHACHA20-POLY1305,ECDHE-RSA-CHACHA20-POLY1305,ECDHE-RSA-AES128-SHA,ECDHE-RSA-AES256-SHA,AES128-GCM-SHA256,AES256-GCM-SHA384,AES128-SHA,AES256-SHA");
            }
            ExternalCurlKind::Generic => {
                command.arg("--http2").arg("--compressed");
            }
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ExternalCurlKind {
    Generic,
    ChromeImpersonate,
}

fn detect_external_kind(binary: &Path) -> ExternalCurlKind {
    match binary.file_name().and_then(|name| name.to_str()) {
        Some("curl-impersonate-chrome") | Some("curl-impersonate") => {
            ExternalCurlKind::ChromeImpersonate
        }
        _ => ExternalCurlKind::Generic,
    }
}

fn binary_supports_option(binary: &Path, option: &str) -> bool {
    Command::new(binary)
        .arg("--help")
        .arg("all")
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .output()
        .ok()
        .and_then(|output| String::from_utf8(output.stdout).ok())
        .map(|text| text.contains(option))
        .unwrap_or(false)
}

fn configure_easy_browser_profile(easy: &mut Easy) {
    let _ = easy.http_version(HttpVersion::V2TLS);
    let _ = easy.accept_encoding("");
    let _ = easy.http_content_decoding(true);
}

fn parse_curl_headers(raw: &[u8]) -> anyhow::Result<(u32, BTreeMap<String, String>)> {
    let text = String::from_utf8_lossy(raw);
    let mut last_status = None;
    let mut current_status = None;
    let mut last_headers = BTreeMap::new();
    let mut current_headers = BTreeMap::new();

    for line in text.lines() {
        let line = line.trim_end_matches('\r');
        if line.is_empty() {
            if let Some(status) = current_status.take() {
                last_status = Some(status);
                last_headers = current_headers.clone();
                current_headers.clear();
            }
            continue;
        }
        if let Some(rest) = line.strip_prefix("HTTP/") {
            let status = rest
                .split_whitespace()
                .next()
                .and_then(|_| line.split_whitespace().nth(1))
                .and_then(|value| value.parse::<u32>().ok())
                .context("missing HTTP status in curl header dump")?;
            current_status = Some(status);
            current_headers.clear();
            continue;
        }
        if current_status.is_some() {
            if let Some((name, value)) = line.split_once(':') {
                current_headers.insert(name.trim().to_ascii_lowercase(), value.trim().to_owned());
            }
        }
    }

    if let Some(status) = current_status {
        last_status = Some(status);
        last_headers = current_headers;
    }

    Ok((last_status.context("no HTTP status in curl header dump")?, last_headers))
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

fn find_in_path(name: &str) -> Option<PathBuf> {
    let path = env::var_os("PATH")?;
    env::split_paths(&path)
        .map(|dir| dir.join(name))
        .find(|candidate| is_executable(candidate))
}

fn is_executable(path: &Path) -> bool {
    path.is_file()
}

fn read_env_nonempty(name: &str) -> Option<String> {
    env::var(name).ok().map(|value| value.trim().to_owned()).filter(|value| !value.is_empty())
}

#[cfg(test)]
mod tests {
    use super::parse_curl_headers;

    #[test]
    fn parses_last_curl_header_block() {
        let raw = b"HTTP/1.1 200 Connection established\r\n\r\nHTTP/2 429\r\nretry-after: 2\r\ncontent-type: application/json\r\n\r\n";
        let (status, headers) = parse_curl_headers(raw).expect("header parsing should succeed");
        assert_eq!(status, 429);
        assert_eq!(headers.get("retry-after").map(String::as_str), Some("2"));
        assert_eq!(
            headers.get("content-type").map(String::as_str),
            Some("application/json")
        );
    }
}
