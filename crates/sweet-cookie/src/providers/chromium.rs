use std::collections::HashSet;
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::Duration;

use aes::Aes128;
use cbc::cipher::{block_padding::Pkcs7, BlockDecryptMut, KeyIvInit};
#[cfg(test)]
use cbc::cipher::BlockEncryptMut;
use pbkdf2::pbkdf2_hmac;
use rusqlite::Connection;
use sha1::Sha1;
use tempfile::tempdir;
use url::Url;
use wait_timeout::ChildExt;

use crate::util::{
    cookie_matches_hosts, copy_sidecar, dedupe_cookies, expand_path, hosts_from_origins,
    looks_like_path, normalize_expiration,
};
use crate::{BrowserName, Cookie, CookieSameSite, CookieSourceInfo, GetCookiesResult};

type Aes128CbcDec = cbc::Decryptor<Aes128>;
#[cfg(test)]
type Aes128CbcEnc = cbc::Encryptor<Aes128>;

pub(crate) fn get_cookies_from_chromium(
    browser: BrowserName,
    profile: Option<String>,
    origins: &[Url],
    allowlist_names: Option<&HashSet<String>>,
    include_expired: bool,
    timeout: Option<Duration>,
) -> anyhow::Result<GetCookiesResult> {
    let Some(db_path) = resolve_chromium_cookies_db(browser, profile.as_deref()) else {
        let label = match browser {
            BrowserName::Chrome => "Chrome",
            BrowserName::Edge => "Edge",
            _ => "Chromium",
        };
        return Ok(GetCookiesResult {
            cookies: Vec::new(),
            warnings: vec![format!("{label} cookies database not found.")],
        });
    };

    let temp_dir = tempdir()?;
    let temp_db_path = temp_dir.path().join("Cookies");
    if let Err(error) = fs::copy(&db_path, &temp_db_path) {
        return Ok(GetCookiesResult {
            cookies: Vec::new(),
            warnings: vec![format!("Failed to copy Chromium cookie DB: {error}")],
        });
    }
    let _ = copy_sidecar(&db_path, &temp_db_path, "-wal");
    let _ = copy_sidecar(&db_path, &temp_db_path, "-shm");

    let password = match read_keychain_password(browser, timeout.unwrap_or(Duration::from_secs(3))) {
        Ok(password) => password,
        Err(error) => {
            return Ok(GetCookiesResult {
                cookies: Vec::new(),
                warnings: vec![error],
            })
        }
    };
    if password.trim().is_empty() {
        return Ok(GetCookiesResult {
            cookies: Vec::new(),
            warnings: vec!["macOS Keychain returned an empty Chromium Safe Storage password.".to_owned()],
        });
    }

    let cookies = read_chromium_sqlite_db(
        &temp_db_path,
        browser,
        profile,
        origins,
        allowlist_names,
        include_expired,
        password.trim(),
    )?;
    Ok(GetCookiesResult {
        cookies,
        warnings: Vec::new(),
    })
}

fn read_chromium_sqlite_db(
    db_path: &Path,
    browser: BrowserName,
    profile: Option<String>,
    origins: &[Url],
    allowlist_names: Option<&HashSet<String>>,
    include_expired: bool,
    safe_storage_password: &str,
) -> anyhow::Result<Vec<Cookie>> {
    let connection = Connection::open(db_path)?;
    let hosts = hosts_from_origins(origins);
    let where_clause = build_host_where_clause(&hosts, "host_key");
    let meta_version = read_chromium_meta_version(&connection).unwrap_or_default();
    let strip_hash_prefix = meta_version >= 24;
    let sql = format!(
        "SELECT name, value, host_key, path, expires_utc, samesite, encrypted_value, \
         is_secure, is_httponly FROM cookies WHERE ({where_clause}) ORDER BY expires_utc DESC"
    );
    let mut statement = connection.prepare(&sql)?;
    let rows = statement.query_map([], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, Option<String>>(1)?,
            row.get::<_, String>(2)?,
            row.get::<_, String>(3)?,
            row.get::<_, Option<i64>>(4)?,
            row.get::<_, Option<i64>>(5)?,
            row.get::<_, Option<Vec<u8>>>(6)?,
            row.get::<_, i64>(7).unwrap_or_default(),
            row.get::<_, i64>(8).unwrap_or_default(),
        ))
    })?;

    let key = derive_mac_key(safe_storage_password);
    let now = unix_now();
    let mut cookies = Vec::new();
    for row in rows {
        let (
            name,
            value,
            host_key,
            path,
            expires_utc,
            same_site,
            encrypted_value,
            is_secure,
            is_http_only,
        ) = row?;

        if !allowlist_names
            .map(|allowlist| allowlist.contains(&name))
            .unwrap_or(true)
        {
            continue;
        }
        if !cookie_matches_hosts(&host_key, &hosts) {
            continue;
        }
        let mut resolved_value = value.unwrap_or_default();
        if resolved_value.is_empty() {
            let Some(encrypted_value) = encrypted_value.as_deref() else {
                continue;
            };
            let Some(decrypted) =
                decrypt_chromium_cookie_value(encrypted_value, &key, strip_hash_prefix)
            else {
                continue;
            };
            resolved_value = decrypted;
        }
        let expires = normalize_expiration(expires_utc);
        if !include_expired && expires.map(|expires| expires < now).unwrap_or(false) {
            continue;
        }
        cookies.push(Cookie {
            name,
            value: resolved_value,
            domain: Some(host_key.trim_start_matches('.').to_owned()),
            path: Some(if path.is_empty() { "/".to_owned() } else { path }),
            url: None,
            expires,
            secure: is_secure == 1,
            http_only: is_http_only == 1,
            same_site: normalize_same_site(same_site),
            source: Some(CookieSourceInfo {
                browser,
                profile: profile.clone(),
                origin: None,
                store_id: None,
            }),
        });
    }
    Ok(dedupe_cookies(cookies))
}

fn resolve_chromium_cookies_db(browser: BrowserName, profile: Option<&str>) -> Option<PathBuf> {
    if let Some(profile) = profile {
        if looks_like_path(profile) {
            let path = expand_path(profile);
            if path.is_file() {
                return Some(path);
            }
            for candidate in [path.join("Cookies"), path.join("Network/Cookies")] {
                if candidate.exists() {
                    return Some(candidate);
                }
            }
        }
    }

    let home = dirs::home_dir()?;
    let roots = match (std::env::consts::OS, browser) {
        ("macos", BrowserName::Chrome) => vec![home.join("Library/Application Support/Google/Chrome")],
        ("macos", BrowserName::Edge) => vec![home.join("Library/Application Support/Microsoft Edge")],
        _ => Vec::new(),
    };
    let profile_dir = profile.unwrap_or("Default");
    for root in roots {
        for candidate in [root.join(profile_dir).join("Cookies"), root.join(profile_dir).join("Network/Cookies")] {
            if candidate.exists() {
                return Some(candidate);
            }
        }
    }
    None
}

fn read_keychain_password(browser: BrowserName, timeout: Duration) -> Result<String, String> {
    let (account, service, label) = match browser {
        BrowserName::Chrome => ("Chrome", "Chrome Safe Storage", "Chrome Safe Storage"),
        BrowserName::Edge => (
            "Microsoft Edge",
            "Microsoft Edge Safe Storage",
            "Microsoft Edge Safe Storage",
        ),
        _ => ("Chromium", "Chromium Safe Storage", "Chromium Safe Storage"),
    };
    read_keychain_generic_password(account, service, label, timeout)
}

fn read_keychain_generic_password(
    account: &str,
    service: &str,
    label: &str,
    timeout: Duration,
) -> Result<String, String> {
    let mut child = Command::new("security")
        .args(["find-generic-password", "-w", "-a", account, "-s", service])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|error| format!("Failed to read macOS Keychain ({label}): {error}"))?;
    let status = child
        .wait_timeout(timeout)
        .map_err(|error| format!("Failed to read macOS Keychain ({label}): {error}"))?
        .ok_or_else(|| format!("Failed to read macOS Keychain ({label}): timed out"))?;
    let mut stdout = String::new();
    let mut stderr = String::new();
    if let Some(mut pipe) = child.stdout.take() {
        let _ = pipe.read_to_string(&mut stdout);
    }
    if let Some(mut pipe) = child.stderr.take() {
        let _ = pipe.read_to_string(&mut stderr);
    }
    if status.success() {
        Ok(stdout.trim().to_owned())
    } else {
        Err(format!(
            "Failed to read macOS Keychain ({label}): {}",
            stderr.trim().if_empty_then("permission denied / keychain locked / entry missing.")
        ))
    }
}

fn derive_mac_key(password: &str) -> [u8; 16] {
    let mut key = [0u8; 16];
    pbkdf2_hmac::<Sha1>(password.as_bytes(), b"saltysalt", 1003, &mut key);
    key
}

fn decrypt_chromium_cookie_value(
    encrypted_value: &[u8],
    key: &[u8; 16],
    strip_hash_prefix: bool,
) -> Option<String> {
    if encrypted_value.len() < 3 {
        return None;
    }
    let payload = if encrypted_value.starts_with(b"v10") || encrypted_value.starts_with(b"v11") {
        let iv = [0x20u8; 16];
        Aes128CbcDec::new_from_slices(key, &iv)
            .ok()?
            .decrypt_padded_vec_mut::<Pkcs7>(&encrypted_value[3..])
            .ok()?
    } else {
        encrypted_value.to_vec()
    };
    let payload = if strip_hash_prefix && payload.len() >= 32 {
        payload[32..].to_vec()
    } else {
        payload
    };
    let value = String::from_utf8(payload).ok()?;
    Some(value.trim_start_matches(|char: char| char.is_control()).to_owned())
}

fn read_chromium_meta_version(connection: &Connection) -> Option<i64> {
    connection
        .query_row(
            "SELECT value FROM meta WHERE key = 'version'",
            [],
            |row| row.get::<_, String>(0),
        )
        .ok()
        .and_then(|value| value.parse::<i64>().ok())
}

fn build_host_where_clause(hosts: &[String], column: &str) -> String {
    let mut clauses = Vec::new();
    for host in hosts {
        let escaped = host.replace('\'', "''");
        clauses.push(format!("{column} = '{escaped}'"));
        clauses.push(format!("{column} = '.{escaped}'"));
        clauses.push(format!("{column} LIKE '%.{escaped}'"));
    }
    if clauses.is_empty() {
        "1=0".to_owned()
    } else {
        clauses.join(" OR ")
    }
}

fn normalize_same_site(value: Option<i64>) -> Option<CookieSameSite> {
    match value {
        Some(2) => Some(CookieSameSite::Strict),
        Some(1) => Some(CookieSameSite::Lax),
        Some(0) => Some(CookieSameSite::None),
        _ => None,
    }
}

fn unix_now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_secs() as i64)
        .unwrap_or_default()
}

trait EmptyFallback {
    fn if_empty_then<'a>(&'a self, fallback: &'a str) -> &'a str;
}

impl EmptyFallback for str {
    fn if_empty_then<'a>(&'a self, fallback: &'a str) -> &'a str {
        if self.trim().is_empty() {
            fallback
        } else {
            self
        }
    }
}

#[cfg(test)]
mod tests {
    use rusqlite::Connection;
    use tempfile::tempdir;
    use url::Url;

    use super::*;

    #[test]
    fn decrypts_a_chromium_cookie_row() {
        let temp_dir = tempdir().unwrap();
        let db_path = temp_dir.path().join("Cookies");
        let connection = Connection::open(&db_path).unwrap();
        connection
            .execute_batch(
                "CREATE TABLE meta (key TEXT PRIMARY KEY, value TEXT);
                 CREATE TABLE cookies (
                    creation_utc INTEGER,
                    host_key TEXT,
                    name TEXT,
                    value TEXT,
                    path TEXT,
                    expires_utc INTEGER,
                    is_secure INTEGER,
                    is_httponly INTEGER,
                    last_access_utc INTEGER,
                    has_expires INTEGER,
                    is_persistent INTEGER,
                    priority INTEGER,
                    encrypted_value BLOB,
                    samesite INTEGER
                 );",
            )
            .unwrap();
        connection
            .execute(
                "INSERT INTO meta (key, value) VALUES ('version', '23')",
                [],
            )
            .unwrap();
        let key = derive_mac_key("test-password");
        let encrypted_value = encrypt_test_cookie("abc123", &key);
        connection
            .execute(
                "INSERT INTO cookies (host_key, name, value, path, expires_utc, is_secure, is_httponly, encrypted_value, samesite)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
                rusqlite::params![
                    ".x.com",
                    "auth_token",
                    "",
                    "/",
                    9_999_999_999_i64,
                    1,
                    1,
                    encrypted_value,
                    1
                ],
            )
            .unwrap();
        drop(connection);

        let cookies = read_chromium_sqlite_db(
            &db_path,
            BrowserName::Chrome,
            Some("Default".to_owned()),
            &[Url::parse("https://x.com/").unwrap()],
            None,
            true,
            "test-password",
        )
        .unwrap();
        assert_eq!(cookies.len(), 1);
        assert_eq!(cookies[0].value, "abc123");
    }

    fn encrypt_test_cookie(value: &str, key: &[u8; 16]) -> Vec<u8> {
        let iv = [0x20u8; 16];
        let mut payload = b"v10".to_vec();
        payload.extend_from_slice(
            &Aes128CbcEnc::new_from_slices(key, &iv)
                .unwrap()
                .encrypt_padded_vec_mut::<Pkcs7>(value.as_bytes()),
        );
        payload
    }
}
