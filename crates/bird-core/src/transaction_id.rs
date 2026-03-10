use std::sync::Mutex;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use anyhow::{Context, bail};
use base64::Engine;
use rand::Rng;
use regex::Regex;
use scraper::{ElementRef, Html, Selector};
use sha2::{Digest, Sha256};
use url::{Url, form_urlencoded::Serializer};

use crate::transport::{HttpRequest, HttpTransport};

const DEFAULT_TTL: Duration = Duration::from_secs(30 * 60);
const X_TRANSACTION_URL: &str = "https://x.com";
const DEFAULT_KEYWORD: &str = "obfiowerehiring";
const ADDITIONAL_RANDOM_NUMBER: u8 = 3;
const TOTAL_TIME: f64 = 4096.0;
const TRANSACTION_EPOCH_SECONDS: u64 = 1_682_924_400;

#[derive(Debug, Clone)]
struct TransactionState {
    key_bytes: Vec<u8>,
    animation_key: String,
}

impl TransactionState {
    fn generate(&self, method: &str, path: &str, time_now: u32, random_byte: u8) -> String {
        let data = format!(
            "{}!{}!{}{}{}",
            method.to_ascii_uppercase(),
            path,
            time_now,
            DEFAULT_KEYWORD,
            self.animation_key
        );
        let hash = Sha256::digest(data.as_bytes());
        let time_now_bytes = [
            (time_now & 0xff) as u8,
            ((time_now >> 8) & 0xff) as u8,
            ((time_now >> 16) & 0xff) as u8,
            ((time_now >> 24) & 0xff) as u8,
        ];
        let mut bytes = Vec::with_capacity(1 + self.key_bytes.len() + time_now_bytes.len() + 16 + 1);
        bytes.push(random_byte);
        for value in self
            .key_bytes
            .iter()
            .copied()
            .chain(time_now_bytes)
            .chain(hash[..16].iter().copied())
            .chain(std::iter::once(ADDITIONAL_RANDOM_NUMBER))
        {
            bytes.push(value ^ random_byte);
        }
        base64::engine::general_purpose::STANDARD_NO_PAD.encode(bytes)
    }
}

#[derive(Debug, Clone)]
struct CachedTransactionState {
    created_at: Instant,
    state: TransactionState,
}

#[derive(Debug)]
pub struct RuntimeTransactionIdStore {
    ttl: Duration,
    cached: Mutex<Option<CachedTransactionState>>,
}

impl Default for RuntimeTransactionIdStore {
    fn default() -> Self {
        Self::new(Some(DEFAULT_TTL))
    }
}

impl RuntimeTransactionIdStore {
    pub fn new(ttl: Option<Duration>) -> Self {
        Self {
            ttl: ttl.unwrap_or(DEFAULT_TTL),
            cached: Mutex::new(None),
        }
    }

    pub fn generate(
        &self,
        transport: &dyn HttpTransport,
        user_agent: &str,
        method: &str,
        url: &str,
    ) -> anyhow::Result<String> {
        let path = Url::parse(url)
            .map(|url| url.path().to_owned())
            .unwrap_or_else(|_| url.to_owned());
        let state = self.get_or_init(transport, user_agent)?;
        let now = current_transaction_time();
        let random_byte = rand::rng().random::<u8>();
        Ok(state.generate(method, &path, now, random_byte))
    }

    fn get_or_init(
        &self,
        transport: &dyn HttpTransport,
        user_agent: &str,
    ) -> anyhow::Result<TransactionState> {
        if let Some(state) = self.cached.lock().ok().and_then(|guard| guard.clone()) {
            if state.created_at.elapsed() <= self.ttl {
                return Ok(state.state);
            }
        }

        let state = fetch_transaction_state(transport, user_agent)?;
        if let Ok(mut guard) = self.cached.lock() {
            *guard = Some(CachedTransactionState {
                created_at: Instant::now(),
                state: state.clone(),
            });
        }
        Ok(state)
    }
}

fn fetch_transaction_state(
    transport: &dyn HttpTransport,
    user_agent: &str,
) -> anyhow::Result<TransactionState> {
    let html = fetch_transaction_document(transport, user_agent)?;
    let token = extract_ondemand_token(&html)?;
    let ondemand_url = format!(
        "https://abs.twimg.com/responsive-web/client-web/ondemand.s.{token}a.js"
    );
    let ondemand_js = fetch_text(transport, "GET", &ondemand_url, vec![], None)?;
    let (row_index, key_byte_indices) = extract_indices(&ondemand_js)?;
    let key_bytes = extract_site_verification_key_bytes(&html)?;
    let frame_paths = extract_frame_paths(&html)?;
    let animation_key = build_animation_key(&key_bytes, row_index, &key_byte_indices, &frame_paths)?;
    Ok(TransactionState {
        key_bytes,
        animation_key,
    })
}

fn fetch_transaction_document(
    transport: &dyn HttpTransport,
    user_agent: &str,
) -> anyhow::Result<String> {
    let browser_headers = transaction_document_headers(user_agent);
    let mut html = fetch_text(
        transport,
        "GET",
        X_TRANSACTION_URL,
        browser_headers.clone(),
        None,
    )?;

    if let Some(redirect_url) = extract_migration_redirect_url(&html) {
        html = fetch_text(transport, "GET", &redirect_url, browser_headers.clone(), None)?;
    }

    if let Some(form) = extract_migration_form(&html)? {
        let action_url = resolve_url(X_TRANSACTION_URL, &form.action)?;
        let body = form.body.into_bytes();
        let mut headers = browser_headers;
        headers.push((
            "content-type".into(),
            "application/x-www-form-urlencoded".into(),
        ));
        if form.method.eq_ignore_ascii_case("GET") {
            let query = String::from_utf8(body).unwrap_or_default();
            let url = if query.is_empty() {
                action_url
            } else {
                format!("{action_url}?{query}")
            };
            html = fetch_text(transport, "GET", &url, headers, None)?;
        } else {
            html = fetch_text(transport, &form.method, &action_url, headers, Some(body))?;
        }
    }

    Ok(html)
}

fn fetch_text(
    transport: &dyn HttpTransport,
    method: &str,
    url: &str,
    headers: Vec<(String, String)>,
    body: Option<Vec<u8>>,
) -> anyhow::Result<String> {
    let response = transport.send(&HttpRequest {
        method: method.to_owned(),
        url: url.to_owned(),
        headers,
        body,
        timeout: Some(Duration::from_secs(20)),
    })?;
    if !response.is_success() {
        bail!("HTTP {} fetching {}", response.status, url);
    }
    Ok(response.text())
}

fn transaction_document_headers(user_agent: &str) -> Vec<(String, String)> {
    let chrome_version = chrome_version_from_user_agent(user_agent);
    vec![
        (
            "accept".into(),
            "text/html,application/xhtml+xml,application/xml;q=0.9,image/avif,image/webp,image/apng,*/*;q=0.8,application/signed-exchange;v=b3;q=0.7"
                .into(),
        ),
        ("accept-language".into(), "en-US,en;q=0.9".into()),
        ("cache-control".into(), "no-cache".into()),
        ("pragma".into(), "no-cache".into()),
        ("priority".into(), "u=0, i".into()),
        (
            "sec-ch-ua".into(),
            format!(
                "\"Google Chrome\";v=\"{chrome_version}\", \"Not-A.Brand\";v=\"8\", \"Chromium\";v=\"{chrome_version}\""
            ),
        ),
        ("sec-ch-ua-mobile".into(), "?0".into()),
        ("sec-ch-ua-platform".into(), "\"macOS\"".into()),
        ("sec-fetch-dest".into(), "document".into()),
        ("sec-fetch-mode".into(), "navigate".into()),
        ("sec-fetch-site".into(), "none".into()),
        ("sec-fetch-user".into(), "?1".into()),
        ("upgrade-insecure-requests".into(), "1".into()),
        ("user-agent".into(), user_agent.to_owned()),
    ]
}

fn chrome_version_from_user_agent(user_agent: &str) -> String {
    Regex::new(r"Chrome/(\d+)")
        .ok()
        .and_then(|regex| regex.captures(user_agent))
        .and_then(|captures| captures.get(1).map(|value| value.as_str().to_owned()))
        .unwrap_or_else(|| "136".to_owned())
}

fn extract_ondemand_token(html: &str) -> anyhow::Result<String> {
    let regex = Regex::new(r#"['"]ondemand\.s['"]\s*:\s*['"]([\w]*)['"]"#)?;
    regex
        .captures(html)
        .and_then(|captures| captures.get(1))
        .map(|value| value.as_str().to_owned())
        .filter(|value| !value.is_empty())
        .context("Failed to locate ondemand token in x.com document")
}

fn extract_indices(ondemand_js: &str) -> anyhow::Result<(usize, Vec<usize>)> {
    let regex = Regex::new(r#"\(\w\[(\d{1,2})\],\s*16\)"#)?;
    let indices = regex
        .captures_iter(ondemand_js)
        .filter_map(|captures| captures.get(1))
        .filter_map(|value| value.as_str().parse::<usize>().ok())
        .collect::<Vec<_>>();
    if indices.len() < 2 {
        bail!("Could not extract transaction indices from ondemand bundle");
    }
    Ok((indices[0], indices[1..].to_vec()))
}

fn extract_site_verification_key_bytes(html: &str) -> anyhow::Result<Vec<u8>> {
    let document = Html::parse_document(html);
    let selector =
        Selector::parse(r#"meta[name="twitter-site-verification"]"#).expect("valid selector");
    let encoded = document
        .select(&selector)
        .next()
        .and_then(|element| element.value().attr("content"))
        .filter(|value| !value.is_empty())
        .context("Couldn't get twitter-site-verification key from x.com document")?;
    base64::engine::general_purpose::STANDARD
        .decode(encoded)
        .context("Failed to decode twitter-site-verification key")
}

fn extract_frame_paths(html: &str) -> anyhow::Result<Vec<String>> {
    let document = Html::parse_document(html);
    let selector = Selector::parse(r#"[id^="loading-x-anim"]"#).expect("valid selector");
    let frames = document.select(&selector).collect::<Vec<_>>();
    if frames.len() < 4 {
        bail!("Expected loading-x animation frames in x.com document");
    }
    let mut paths = Vec::with_capacity(frames.len());
    for frame in frames {
        let first_child = frame
            .children()
            .filter_map(ElementRef::wrap)
            .next()
            .context("Missing first child in loading-x frame")?;
        let mut inner_children = first_child.children().filter_map(ElementRef::wrap);
        let _ = inner_children.next();
        let target_child = inner_children
            .next()
            .context("Missing target path child in loading-x frame")?;
        let d_attr = target_child
            .value()
            .attr("d")
            .context("Missing path data in loading-x frame")?;
        paths.push(d_attr.to_owned());
    }
    Ok(paths)
}

fn build_animation_key(
    key_bytes: &[u8],
    row_index: usize,
    key_byte_indices: &[usize],
    frame_paths: &[String],
) -> anyhow::Result<String> {
    if key_bytes.len() <= 5 {
        bail!("Verification key is too short");
    }
    let frame_index = key_bytes[5] as usize % 4;
    let target_frame = frame_paths
        .get(frame_index)
        .context("Missing selected loading-x animation frame")?;
    let rows = parse_path_rows(target_frame);
    let selected_row_index = key_bytes
        .get(row_index)
        .copied()
        .context("Missing row selection byte in verification key")? as usize
        % 16;
    let row = rows
        .get(selected_row_index)
        .context("Missing selected animation row")?;
    if row.len() < 11 {
        bail!("Animation row is too short");
    }

    let mut frame_time = 1u64;
    for index in key_byte_indices {
        let value = key_bytes
            .get(*index)
            .copied()
            .with_context(|| format!("Missing key byte index {}", index))?;
        frame_time = frame_time.saturating_mul((value % 16) as u64);
    }
    frame_time = ((frame_time as f64 / 10.0).round() as u64) * 10;
    let target_time = frame_time as f64 / TOTAL_TIME;
    animate_row(row, target_time)
}

fn parse_path_rows(path: &str) -> Vec<Vec<u16>> {
    path.get(9..)
        .unwrap_or_default()
        .split('C')
        .map(|segment| {
            let cleaned = segment
                .chars()
                .map(|ch| if ch.is_ascii_digit() { ch } else { ' ' })
                .collect::<String>();
            cleaned
                .split_whitespace()
                .filter_map(|value| value.parse::<u16>().ok())
                .collect::<Vec<_>>()
        })
        .collect()
}

fn animate_row(row: &[u16], target_time: f64) -> anyhow::Result<String> {
    let from_color = row[0..3]
        .iter()
        .copied()
        .map(|value| value as f64)
        .chain(std::iter::once(1.0))
        .collect::<Vec<_>>();
    let to_color = row[3..6]
        .iter()
        .copied()
        .map(|value| value as f64)
        .chain(std::iter::once(1.0))
        .collect::<Vec<_>>();
    let to_rotation = solve(row[6], 60.0, 360.0, true);
    let curves = row[7..]
        .iter()
        .enumerate()
        .map(|(index, value)| solve(*value, odd_value(index), 1.0, false))
        .collect::<Vec<_>>();
    if curves.len() < 4 {
        bail!("Animation row does not contain cubic bezier curves");
    }

    let cubic = Cubic::new(curves);
    let interpolated = cubic.get_value(target_time);
    let color = interpolate(&from_color, &to_color, interpolated)
        .into_iter()
        .map(|value| value.max(0.0))
        .collect::<Vec<_>>();
    let matrix = convert_rotation_to_matrix(interpolate(&[0.0], &[to_rotation], interpolated)[0]);

    let mut parts = color
        .iter()
        .take(3)
        .map(|value| format!("{:x}", value.round() as i64))
        .collect::<Vec<_>>();
    for value in matrix {
        let mut rounded = (value * 100.0).round() / 100.0;
        if rounded < 0.0 {
            rounded = -rounded;
        }
        let hex = float_to_hex(rounded);
        if hex.starts_with('.') {
            parts.push(format!("0{}", hex.to_lowercase()));
        } else if hex.is_empty() {
            parts.push("0".to_owned());
        } else {
            parts.push(hex.to_lowercase());
        }
    }
    parts.push("0".into());
    parts.push("0".into());
    Ok(parts.join("").replace(['.', '-'], ""))
}

fn solve(value: u16, min_val: f64, max_val: f64, rounding: bool) -> f64 {
    let result = (value as f64 * (max_val - min_val)) / 255.0 + min_val;
    if rounding {
        result.floor()
    } else {
        (result * 100.0).round() / 100.0
    }
}

fn odd_value(index: usize) -> f64 {
    if index % 2 == 1 {
        -1.0
    } else {
        0.0
    }
}

fn interpolate(from_list: &[f64], to_list: &[f64], factor: f64) -> Vec<f64> {
    from_list
        .iter()
        .zip(to_list.iter())
        .map(|(from, to)| from * (1.0 - factor) + to * factor)
        .collect()
}

fn convert_rotation_to_matrix(rotation: f64) -> [f64; 4] {
    let radians = rotation * std::f64::consts::PI / 180.0;
    [radians.cos(), -radians.sin(), radians.sin(), radians.cos()]
}

fn float_to_hex(mut value: f64) -> String {
    let mut result = Vec::new();
    let mut quotient = value.floor();
    let mut fraction = value - quotient;

    while quotient > 0.0 {
        let next_quotient = (value / 16.0).floor();
        let remainder = (value - next_quotient * 16.0).floor() as u8;
        result.insert(0, hex_digit(remainder));
        value = next_quotient;
        quotient = value.floor();
    }

    if fraction == 0.0 {
        return result.into_iter().collect();
    }

    result.push('.');
    while fraction > 0.0 {
        fraction *= 16.0;
        let integer = fraction.floor() as u8;
        fraction -= integer as f64;
        result.push(hex_digit(integer));
    }
    result.into_iter().collect()
}

fn hex_digit(value: u8) -> char {
    match value {
        0..=9 => (b'0' + value) as char,
        10..=15 => (b'A' + (value - 10)) as char,
        _ => '0',
    }
}

#[derive(Debug)]
struct Cubic {
    curves: Vec<f64>,
}

impl Cubic {
    fn new(curves: Vec<f64>) -> Self {
        Self { curves }
    }

    fn get_value(&self, time: f64) -> f64 {
        let mut start = 0.0;
        let mut mid = 0.0;
        let mut end = 1.0;

        if time <= 0.0 {
            let start_gradient = if self.curves[0] > 0.0 {
                self.curves[1] / self.curves[0]
            } else if self.curves[1] == 0.0 && self.curves[2] > 0.0 {
                self.curves[3] / self.curves[2]
            } else {
                0.0
            };
            return start_gradient * time;
        }

        if time >= 1.0 {
            let end_gradient = if self.curves[2] < 1.0 {
                (self.curves[3] - 1.0) / (self.curves[2] - 1.0)
            } else if self.curves[2] == 1.0 && self.curves[0] < 1.0 {
                (self.curves[1] - 1.0) / (self.curves[0] - 1.0)
            } else {
                0.0
            };
            return 1.0 + end_gradient * (time - 1.0);
        }

        while start < end {
            mid = (start + end) / 2.0;
            let estimate = cubic_calculate(self.curves[0], self.curves[2], mid);
            if (time - estimate).abs() < 0.00001 {
                return cubic_calculate(self.curves[1], self.curves[3], mid);
            }
            if estimate < time {
                start = mid;
            } else {
                end = mid;
            }
        }
        cubic_calculate(self.curves[1], self.curves[3], mid)
    }
}

fn cubic_calculate(a: f64, b: f64, m: f64) -> f64 {
    3.0 * a * (1.0 - m) * (1.0 - m) * m + 3.0 * b * (1.0 - m) * m * m + m * m * m
}

#[derive(Debug)]
struct MigrationForm {
    action: String,
    method: String,
    body: String,
}

fn extract_migration_redirect_url(html: &str) -> Option<String> {
    let regex = Regex::new(
        r#"(http(?:s)?://(?:www\.)?(twitter|x){1}\.com(/x)?/migrate([/?])?tok=[a-zA-Z0-9%\-_]+)+"#,
    )
    .ok()?;
    let document = Html::parse_document(html);
    let selector = Selector::parse(r#"meta[http-equiv="refresh"]"#).ok()?;
    let meta_content = document
        .select(&selector)
        .next()
        .and_then(|element| element.value().attr("content"))
        .unwrap_or_default();
    regex
        .captures(meta_content)
        .or_else(|| regex.captures(html))
        .and_then(|captures| captures.get(0).map(|value| value.as_str().to_owned()))
}

fn extract_migration_form(html: &str) -> anyhow::Result<Option<MigrationForm>> {
    let document = Html::parse_document(html);
    let form_selector = Selector::parse(r#"form[name="f"], form[action="https://x.com/x/migrate"]"#)
        .expect("valid selector");
    let input_selector = Selector::parse("input").expect("valid selector");
    let Some(form) = document.select(&form_selector).next() else {
        return Ok(None);
    };
    let action = form
        .value()
        .attr("action")
        .unwrap_or("https://x.com/x/migrate")
        .to_owned();
    let method = form.value().attr("method").unwrap_or("POST").to_owned();
    let mut serializer = Serializer::new(String::new());
    for input in form.select(&input_selector) {
        if let (Some(name), Some(value)) = (input.value().attr("name"), input.value().attr("value"))
        {
            serializer.append_pair(name, value);
        }
    }
    Ok(Some(MigrationForm {
        action,
        method,
        body: serializer.finish(),
    }))
}

fn resolve_url(base: &str, url: &str) -> anyhow::Result<String> {
    if Url::parse(url).is_ok() {
        return Ok(url.to_owned());
    }
    Ok(Url::parse(base)?.join(url)?.to_string())
}

fn current_transaction_time() -> u32 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs().saturating_sub(TRANSACTION_EPOCH_SECONDS) as u32)
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::{
        TransactionState, animate_row, extract_indices, extract_ondemand_token, parse_path_rows,
    };

    #[test]
    fn extracts_ondemand_token_from_homepage_html() {
        let html = r#"<script>window.__SCRIPTS__={"ondemand.s":"abc123"}</script>"#;
        assert_eq!(extract_ondemand_token(html).expect("token"), "abc123");
    }

    #[test]
    fn extracts_indices_from_ondemand_bundle() {
        let js = r#"return (a[5], 16), (b[2], 16), (c[7], 16), (d[9], 16);"#;
        let (row_index, indices) = extract_indices(js).expect("indices");
        assert_eq!(row_index, 5);
        assert_eq!(indices, vec![2, 7, 9]);
    }

    #[test]
    fn parses_svg_path_rows() {
        let rows = parse_path_rows("12345678912,34,56,78,90,12C13,14,15,16,17,18");
        assert_eq!(rows[0], vec![12, 34, 56, 78, 90, 12]);
        assert_eq!(rows[1], vec![13, 14, 15, 16, 17, 18]);
    }

    #[test]
    fn generates_deterministic_transaction_id() {
        let state = TransactionState {
            key_bytes: vec![1, 2, 3, 4, 5, 6, 7, 8],
            animation_key: "abcdef1234".into(),
        };
        let value = state.generate("GET", "/i/api/test", 123456, 42);
        assert_eq!(value, "KisoKS4vLC0iasgrKumXZGaA9atwL902kEAn7Zgp");
    }

    #[test]
    fn animation_key_matches_js_port_vector() {
        let row = vec![40, 80, 120, 10, 20, 30, 200, 64, 128, 192, 255];
        let key = animate_row(&row, 0.5).expect("animation");
        assert_eq!(key, "19324b0d70a3d70a3d70808a3d70a3d70a408a3d70a3d70a40d70a3d70a3d70800");
    }
}
