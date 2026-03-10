use std::collections::HashSet;
use std::fs;
use std::path::PathBuf;

use url::Url;

use crate::util::{cookie_matches_hosts, dedupe_cookies, hosts_from_origins, safe_hostname_from_url};
use crate::{Cookie, CookieSourceInfo, GetCookiesResult};

const MAC_EPOCH_DELTA_SECONDS: i64 = 978_307_200;

pub(crate) fn get_cookies_from_safari(
    file_override: Option<PathBuf>,
    origins: &[Url],
    allowlist_names: Option<&HashSet<String>>,
    include_expired: bool,
) -> anyhow::Result<GetCookiesResult> {
    if std::env::consts::OS != "macos" {
        return Ok(GetCookiesResult {
            cookies: Vec::new(),
            warnings: Vec::new(),
        });
    }

    let cookie_file = file_override.or_else(resolve_safari_binary_cookies_path);
    let Some(cookie_file) = cookie_file else {
        return Ok(GetCookiesResult {
            cookies: Vec::new(),
            warnings: vec!["Safari Cookies.binarycookies not found.".to_owned()],
        });
    };

    let data = match fs::read(&cookie_file) {
        Ok(data) => data,
        Err(error) => {
            return Ok(GetCookiesResult {
                cookies: Vec::new(),
                warnings: vec![format!("Failed to read Safari cookies: {error}")],
            });
        }
    };

    let now = unix_now();
    let hosts = hosts_from_origins(origins);
    let cookies = decode_binary_cookies(&data)
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
            domain
                .as_deref()
                .map(|domain| cookie_matches_hosts(domain, &hosts))
                .unwrap_or(false)
        })
        .filter(|cookie| include_expired || cookie.expires.map(|expires| expires >= now).unwrap_or(true))
        .collect::<Vec<_>>();

    Ok(GetCookiesResult {
        cookies: dedupe_cookies(cookies),
        warnings: Vec::new(),
    })
}

fn resolve_safari_binary_cookies_path() -> Option<PathBuf> {
    let home = dirs::home_dir()?;
    let candidates = [
        home.join("Library/Cookies/Cookies.binarycookies"),
        home.join("Library/Containers/com.apple.Safari/Data/Library/Cookies/Cookies.binarycookies"),
    ];
    candidates.into_iter().find(|candidate| candidate.exists())
}

fn decode_binary_cookies(buffer: &[u8]) -> Vec<Cookie> {
    if buffer.len() < 8 || &buffer[0..4] != b"cook" {
        return Vec::new();
    }
    let page_count = u32::from_be_bytes(buffer[4..8].try_into().unwrap_or([0; 4])) as usize;
    let mut cursor = 8usize;
    let mut page_sizes = Vec::new();
    for _ in 0..page_count {
        if cursor + 4 > buffer.len() {
            return Vec::new();
        }
        page_sizes.push(u32::from_be_bytes(buffer[cursor..cursor + 4].try_into().unwrap()) as usize);
        cursor += 4;
    }

    let mut cookies = Vec::new();
    for page_size in page_sizes {
        if cursor + page_size > buffer.len() {
            break;
        }
        cookies.extend(decode_page(&buffer[cursor..cursor + page_size]));
        cursor += page_size;
    }
    cookies
}

fn decode_page(page: &[u8]) -> Vec<Cookie> {
    if page.len() < 8 || u32::from_be_bytes(page[0..4].try_into().unwrap_or([0; 4])) != 0x00000100 {
        return Vec::new();
    }
    let cookie_count = u32::from_le_bytes(page[4..8].try_into().unwrap_or([0; 4])) as usize;
    let mut cursor = 8usize;
    let mut offsets = Vec::new();
    for _ in 0..cookie_count {
        if cursor + 4 > page.len() {
            return Vec::new();
        }
        offsets.push(u32::from_le_bytes(page[cursor..cursor + 4].try_into().unwrap()) as usize);
        cursor += 4;
    }

    offsets
        .into_iter()
        .filter_map(|offset| decode_cookie(page.get(offset..).unwrap_or_default()))
        .collect()
}

fn decode_cookie(cookie_buffer: &[u8]) -> Option<Cookie> {
    if cookie_buffer.len() < 48 {
        return None;
    }
    let size = u32::from_le_bytes(cookie_buffer[0..4].try_into().ok()?) as usize;
    if size < 48 || size > cookie_buffer.len() {
        return None;
    }
    let flags_value = u32::from_le_bytes(cookie_buffer[8..12].try_into().ok()?);
    let is_secure = flags_value & 1 != 0;
    let is_http_only = flags_value & 4 != 0;
    let url_offset = u32::from_le_bytes(cookie_buffer[16..20].try_into().ok()?) as usize;
    let name_offset = u32::from_le_bytes(cookie_buffer[20..24].try_into().ok()?) as usize;
    let path_offset = u32::from_le_bytes(cookie_buffer[24..28].try_into().ok()?) as usize;
    let value_offset = u32::from_le_bytes(cookie_buffer[28..32].try_into().ok()?) as usize;
    let expiration = read_double_le(cookie_buffer, 40)?;
    let raw_url = read_c_string(cookie_buffer, url_offset, size);
    let name = read_c_string(cookie_buffer, name_offset, size)?;
    let path = read_c_string(cookie_buffer, path_offset, size).unwrap_or_else(|| "/".to_owned());
    let value = read_c_string(cookie_buffer, value_offset, size).unwrap_or_default();
    let domain = raw_url
        .as_deref()
        .and_then(safe_hostname_from_url)
        .or_else(|| raw_url.clone());

    Some(Cookie {
        name,
        value,
        domain,
        path: Some(path),
        url: None,
        expires: if expiration > 0.0 {
            Some((expiration.round() as i64) + MAC_EPOCH_DELTA_SECONDS)
        } else {
            None
        },
        secure: is_secure,
        http_only: is_http_only,
        same_site: None,
        source: Some(CookieSourceInfo {
            browser: crate::BrowserName::Safari,
            profile: None,
            origin: None,
            store_id: None,
        }),
    })
}

fn read_double_le(buffer: &[u8], offset: usize) -> Option<f64> {
    let bytes: [u8; 8] = buffer.get(offset..offset + 8)?.try_into().ok()?;
    Some(f64::from_le_bytes(bytes))
}

fn read_c_string(buffer: &[u8], offset: usize, end: usize) -> Option<String> {
    if offset == 0 || offset >= end || end > buffer.len() {
        return None;
    }
    let mut cursor = offset;
    while cursor < end && buffer[cursor] != 0 {
        cursor += 1;
    }
    if cursor > end {
        return None;
    }
    String::from_utf8(buffer[offset..cursor].to_vec()).ok()
}

fn unix_now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_secs() as i64)
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use tempfile::NamedTempFile;
    use url::Url;

    use super::*;

    #[test]
    fn decodes_safari_fixture() {
        let file = NamedTempFile::new().unwrap();
        std::fs::write(file.path(), build_test_binarycookies()).unwrap();
        let result = get_cookies_from_safari(
            Some(file.path().to_path_buf()),
            &[Url::parse("https://x.com/").unwrap()],
            None,
            true,
        )
        .unwrap();
        assert_eq!(result.cookies.len(), 1);
        assert_eq!(result.cookies[0].name, "auth_token");
        assert_eq!(result.cookies[0].domain.as_deref(), Some("x.com"));
    }

    fn build_test_binarycookies() -> Vec<u8> {
        let url = b"https://x.com\0";
        let name = b"auth_token\0";
        let path = b"/\0";
        let value = b"abc123\0";
        let header_size = 56usize;
        let url_offset = header_size;
        let name_offset = url_offset + url.len();
        let path_offset = name_offset + name.len();
        let value_offset = path_offset + path.len();
        let size = value_offset + value.len();

        let mut cookie = vec![0u8; size];
        cookie[0..4].copy_from_slice(&(size as u32).to_le_bytes());
        cookie[8..12].copy_from_slice(&1u32.to_le_bytes());
        cookie[16..20].copy_from_slice(&(url_offset as u32).to_le_bytes());
        cookie[20..24].copy_from_slice(&(name_offset as u32).to_le_bytes());
        cookie[24..28].copy_from_slice(&(path_offset as u32).to_le_bytes());
        cookie[28..32].copy_from_slice(&(value_offset as u32).to_le_bytes());
        cookie[40..48].copy_from_slice(&1000f64.to_le_bytes());
        cookie[url_offset..url_offset + url.len()].copy_from_slice(url);
        cookie[name_offset..name_offset + name.len()].copy_from_slice(name);
        cookie[path_offset..path_offset + path.len()].copy_from_slice(path);
        cookie[value_offset..value_offset + value.len()].copy_from_slice(value);

        let mut page = vec![0u8; 12 + cookie.len()];
        page[0..4].copy_from_slice(&0x00000100u32.to_be_bytes());
        page[4..8].copy_from_slice(&1u32.to_le_bytes());
        page[8..12].copy_from_slice(&12u32.to_le_bytes());
        page[12..12 + cookie.len()].copy_from_slice(&cookie);

        let mut file = Vec::new();
        file.extend_from_slice(b"cook");
        file.extend_from_slice(&1u32.to_be_bytes());
        file.extend_from_slice(&(page.len() as u32).to_be_bytes());
        file.extend_from_slice(&page);
        file
    }
}
