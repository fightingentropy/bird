use std::collections::HashSet;
use std::fs;

use base64::Engine;
use url::Url;

use crate::util::{cookie_matches_hosts, dedupe_cookies, hosts_from_origins, safe_hostname_from_url};
use crate::{Cookie, GetCookiesResult};

#[derive(Debug, Clone)]
pub(crate) struct InlineSource {
    pub(crate) source: String,
    pub(crate) payload: String,
}

#[derive(Debug, serde::Deserialize)]
struct CookieEnvelope {
    cookies: Vec<Cookie>,
}

pub(crate) fn get_cookies_from_inline(
    inline: &InlineSource,
    origins: &[Url],
    allowlist_names: Option<&HashSet<String>>,
) -> anyhow::Result<GetCookiesResult> {
    let raw_payload = if inline.source.ends_with("file")
        || inline.payload.ends_with(".json")
        || inline.payload.ends_with(".base64")
    {
        fs::read_to_string(&inline.payload).unwrap_or_else(|_| inline.payload.clone())
    } else {
        inline.payload.clone()
    };

    let decoded = try_decode_base64_json(&raw_payload).unwrap_or(raw_payload);
    let cookies = parse_cookie_payload(&decoded).unwrap_or_default();
    let hosts = hosts_from_origins(origins);

    let filtered = cookies
        .into_iter()
        .filter(|cookie| !cookie.name.is_empty())
        .filter(|cookie| {
            allowlist_names
                .map(|allowlist| allowlist.contains(&cookie.name))
                .unwrap_or(true)
        })
        .filter(|cookie| {
            let domain = cookie
                .domain
                .clone()
                .or_else(|| cookie.url.as_deref().and_then(safe_hostname_from_url));
            match domain {
                Some(domain) => cookie_matches_hosts(&domain, &hosts),
                None => true,
            }
        })
        .collect::<Vec<_>>();

    Ok(GetCookiesResult {
        cookies: dedupe_cookies(filtered),
        warnings: Vec::new(),
    })
}

fn try_decode_base64_json(raw: &str) -> Option<String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return None;
    }
    let bytes = base64::engine::general_purpose::STANDARD.decode(trimmed).ok()?;
    let decoded = String::from_utf8(bytes).ok()?;
    serde_json::from_str::<serde_json::Value>(&decoded).ok()?;
    Some(decoded)
}

fn parse_cookie_payload(input: &str) -> Option<Vec<Cookie>> {
    let trimmed = input.trim();
    if trimmed.is_empty() {
        return None;
    }
    serde_json::from_str::<Vec<Cookie>>(trimmed)
        .ok()
        .or_else(|| {
            serde_json::from_str::<CookieEnvelope>(trimmed)
                .ok()
                .map(|envelope| envelope.cookies)
        })
}

#[cfg(test)]
mod tests {
    use std::io::Write;

    use base64::Engine;
    use tempfile::NamedTempFile;
    use url::Url;

    use super::*;

    #[test]
    fn filters_inline_json_by_origin_and_name() {
        let inline = InlineSource {
            source: "inline-json".to_owned(),
            payload: r#"{"cookies":[{"name":"auth_token","value":"a","domain":"x.com"},{"name":"sid","value":"b","domain":"example.com"}]}"#.to_owned(),
        };
        let result = get_cookies_from_inline(
            &inline,
            &[Url::parse("https://x.com/").unwrap()],
            Some(&HashSet::from(["auth_token".to_owned()])),
        )
        .unwrap();
        assert_eq!(result.cookies.len(), 1);
        assert_eq!(result.cookies[0].name, "auth_token");
    }

    #[test]
    fn decodes_base64_payloads() {
        let payload = base64::engine::general_purpose::STANDARD.encode(
            r#"[{"name":"ct0","value":"123","domain":"x.com"}]"#,
        );
        let result = get_cookies_from_inline(
            &InlineSource {
                source: "inline-base64".to_owned(),
                payload,
            },
            &[Url::parse("https://x.com/").unwrap()],
            None,
        )
        .unwrap();
        assert_eq!(result.cookies.len(), 1);
        assert_eq!(result.cookies[0].name, "ct0");
    }

    #[test]
    fn reads_file_payloads() {
        let mut file = NamedTempFile::new().unwrap();
        write!(
            file,
            r#"[{{"name":"guest_id","value":"1","domain":"x.com"}}]"#
        )
        .unwrap();
        let result = get_cookies_from_inline(
            &InlineSource {
                source: "inline-file".to_owned(),
                payload: file.path().to_string_lossy().into_owned(),
            },
            &[Url::parse("https://x.com/").unwrap()],
            None,
        )
        .unwrap();
        assert_eq!(result.cookies[0].name, "guest_id");
    }
}
