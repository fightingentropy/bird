use std::collections::HashSet;
use std::fs;

use rusqlite::Connection;
use tempfile::tempdir;
use url::Url;

use crate::util::{
    cookie_matches_hosts, copy_sidecar, dedupe_cookies, expand_path, hosts_from_origins,
    looks_like_path,
};
use crate::{BrowserName, Cookie, CookieSameSite, CookieSourceInfo, GetCookiesResult};

pub(crate) fn get_cookies_from_firefox(
    profile: Option<String>,
    origins: &[Url],
    allowlist_names: Option<&HashSet<String>>,
    include_expired: bool,
) -> anyhow::Result<GetCookiesResult> {
    let Some(db_path) = resolve_firefox_cookies_db(profile.as_deref()) else {
        return Ok(GetCookiesResult {
            cookies: Vec::new(),
            warnings: vec!["Firefox cookies database not found.".to_owned()],
        });
    };

    let temp_dir = tempdir()?;
    let temp_db_path = temp_dir.path().join("cookies.sqlite");
    if let Err(error) = fs::copy(&db_path, &temp_db_path) {
        return Ok(GetCookiesResult {
            cookies: Vec::new(),
            warnings: vec![format!("Failed to copy Firefox cookie DB: {error}")],
        });
    }
    let _ = copy_sidecar(&db_path, &temp_db_path, "-wal");
    let _ = copy_sidecar(&db_path, &temp_db_path, "-shm");

    let cookies = read_firefox_sqlite_db(
        &temp_db_path,
        profile,
        origins,
        allowlist_names,
        include_expired,
    )?;

    Ok(GetCookiesResult {
        cookies,
        warnings: Vec::new(),
    })
}

fn read_firefox_sqlite_db(
    db_path: &std::path::Path,
    profile: Option<String>,
    origins: &[Url],
    allowlist_names: Option<&HashSet<String>>,
    include_expired: bool,
) -> anyhow::Result<Vec<Cookie>> {
    let connection = Connection::open(db_path)?;
    let hosts = hosts_from_origins(origins);
    let now = unix_now();
    let where_clause = build_host_where_clause(&hosts);
    let expiry_clause = if include_expired {
        String::new()
    } else {
        format!(" AND (expiry = 0 OR expiry > {now})")
    };
    let sql = format!(
        "SELECT name, value, host, path, expiry, isSecure, isHttpOnly, sameSite \
         FROM moz_cookies WHERE ({where_clause}){expiry_clause} ORDER BY expiry DESC"
    );
    let mut statement = connection.prepare(&sql)?;
    let rows = statement.query_map([], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, String>(2)?,
            row.get::<_, String>(3)?,
            row.get::<_, i64>(4).ok(),
            row.get::<_, i64>(5).unwrap_or_default(),
            row.get::<_, i64>(6).unwrap_or_default(),
            row.get::<_, i64>(7).ok(),
        ))
    })?;

    let mut cookies = Vec::new();
    for row in rows {
        let (name, value, host, path, expiry, is_secure, is_http_only, same_site) = row?;
        if !allowlist_names
            .map(|allowlist| allowlist.contains(&name))
            .unwrap_or(true)
        {
            continue;
        }
        if !cookie_matches_hosts(&host, &hosts) {
            continue;
        }
        let expires = expiry.filter(|expiry| *expiry > 0);
        if !include_expired && expires.map(|expiry| expiry < now).unwrap_or(false) {
            continue;
        }
        cookies.push(Cookie {
            name,
            value,
            domain: Some(host.trim_start_matches('.').to_owned()),
            path: Some(if path.is_empty() {
                "/".to_owned()
            } else {
                path
            }),
            url: None,
            expires,
            secure: is_secure == 1,
            http_only: is_http_only == 1,
            same_site: normalize_same_site(same_site),
            source: Some(CookieSourceInfo {
                browser: BrowserName::Firefox,
                profile: profile.clone(),
                origin: None,
                store_id: None,
            }),
        });
    }
    Ok(dedupe_cookies(cookies))
}

fn resolve_firefox_cookies_db(profile: Option<&str>) -> Option<std::path::PathBuf> {
    if let Some(profile) = profile
        && looks_like_path(profile)
    {
        let path = expand_path(profile);
        if path.is_file() {
            return Some(path);
        }
        let candidate = path.join("cookies.sqlite");
        if candidate.exists() {
            return Some(candidate);
        }
    }

    let home = dirs::home_dir()?;
    let roots = match std::env::consts::OS {
        "macos" => vec![home.join("Library/Application Support/Firefox/Profiles")],
        "linux" => vec![home.join(".mozilla/firefox")],
        _ => Vec::new(),
    };
    for root in roots {
        if !root.exists() {
            continue;
        }
        if let Some(profile) = profile {
            let candidate = root.join(profile).join("cookies.sqlite");
            if candidate.exists() {
                return Some(candidate);
            }
            continue;
        }
        let ini_dir = match root.file_name().and_then(|name| name.to_str()) {
            Some("Profiles") => root.parent().unwrap_or(&root).to_path_buf(),
            _ => root.clone(),
        };
        if let Some(candidate) = pick_default_cookies_db(&root, &ini_dir) {
            return Some(candidate);
        }
    }
    None
}

/// Pick the cookies database of the profile Firefox itself uses: the default
/// named by profiles.ini, else the profile whose cookies.sqlite changed most
/// recently.
fn pick_default_cookies_db(
    profiles_root: &std::path::Path,
    ini_dir: &std::path::Path,
) -> Option<std::path::PathBuf> {
    if let Ok(ini) = fs::read_to_string(ini_dir.join("profiles.ini"))
        && let Some(profile_dir) = default_profile_dir_from_ini(&ini, ini_dir)
    {
        let candidate = profile_dir.join("cookies.sqlite");
        if candidate.exists() {
            return Some(candidate);
        }
    }

    let mut newest: Option<(std::time::SystemTime, String, std::path::PathBuf)> = None;
    for entry in fs::read_dir(profiles_root).ok()?.flatten() {
        if !entry.file_type().map(|kind| kind.is_dir()).unwrap_or(false) {
            continue;
        }
        let candidate = entry.path().join("cookies.sqlite");
        let Ok(modified) = fs::metadata(&candidate).and_then(|meta| meta.modified()) else {
            continue;
        };
        let name = entry.file_name().to_string_lossy().into_owned();
        let better = match &newest {
            None => true,
            Some((best_time, best_name, _)) => {
                modified > *best_time || (modified == *best_time && name < *best_name)
            }
        };
        if better {
            newest = Some((modified, name, candidate));
        }
    }
    newest.map(|(_, _, path)| path)
}

/// The default profile directory according to profiles.ini: the `Default=` of
/// an `[Install*]` section when present, else the `[Profile*]` marked
/// `Default=1`.
fn default_profile_dir_from_ini(
    ini: &str,
    ini_dir: &std::path::Path,
) -> Option<std::path::PathBuf> {
    #[derive(Default)]
    struct Section {
        name: String,
        path: Option<String>,
        is_relative: bool,
        is_default: bool,
        default: Option<String>,
    }
    let mut sections: Vec<Section> = Vec::new();
    for line in ini.lines() {
        let line = line.trim();
        if let Some(name) = line
            .strip_prefix('[')
            .and_then(|rest| rest.strip_suffix(']'))
        {
            sections.push(Section {
                name: name.to_owned(),
                is_relative: true,
                ..Section::default()
            });
            continue;
        }
        let (Some(section), Some((key, value))) = (sections.last_mut(), line.split_once('='))
        else {
            continue;
        };
        let value = value.trim();
        match key.trim() {
            "Path" => section.path = Some(value.to_owned()),
            "IsRelative" => section.is_relative = value != "0",
            "Default" if section.name.starts_with("Install") => {
                section.default = Some(value.to_owned());
            }
            "Default" => section.is_default = value == "1",
            _ => {}
        }
    }

    let resolve = |path: &str, is_relative: bool| {
        if is_relative {
            ini_dir.join(path)
        } else {
            std::path::PathBuf::from(path)
        }
    };
    if let Some(path) = sections
        .iter()
        .filter(|section| section.name.starts_with("Install"))
        .find_map(|section| section.default.as_deref())
    {
        // Install sections always store the path relative to profiles.ini.
        return Some(resolve(path, true));
    }
    sections
        .iter()
        .filter(|section| section.name.starts_with("Profile") && section.is_default)
        .find_map(|section| {
            section
                .path
                .as_deref()
                .map(|path| resolve(path, section.is_relative))
        })
}

fn build_host_where_clause(hosts: &[String]) -> String {
    let mut clauses = Vec::new();
    for host in hosts {
        let escaped = host.replace('\'', "''");
        clauses.push(format!("host = '{escaped}'"));
        clauses.push(format!("host = '.{escaped}'"));
        clauses.push(format!("host LIKE '%.{escaped}'"));
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

#[cfg(test)]
mod tests {
    use rusqlite::Connection;
    use tempfile::tempdir;
    use url::Url;

    use super::*;

    #[test]
    fn reads_matching_firefox_cookie() {
        let temp_dir = tempdir().unwrap();
        let db_path = temp_dir.path().join("cookies.sqlite");
        let connection = Connection::open(&db_path).unwrap();
        connection
            .execute_batch(
                "CREATE TABLE moz_cookies (
                    id INTEGER PRIMARY KEY,
                    name TEXT,
                    value TEXT,
                    host TEXT,
                    path TEXT,
                    expiry INTEGER,
                    isSecure INTEGER,
                    isHttpOnly INTEGER,
                    sameSite INTEGER
                );",
            )
            .unwrap();
        connection
            .execute(
                "INSERT INTO moz_cookies (name, value, host, path, expiry, isSecure, isHttpOnly, sameSite)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
                rusqlite::params![
                    "auth_token",
                    "abc",
                    ".x.com",
                    "/",
                    unix_now() + 600,
                    1,
                    1,
                    1
                ],
            )
            .unwrap();
        drop(connection);

        let cookies = read_firefox_sqlite_db(
            &db_path,
            Some("test-profile".to_owned()),
            &[Url::parse("https://x.com/").unwrap()],
            None,
            false,
        )
        .unwrap();
        assert_eq!(cookies.len(), 1);
        assert_eq!(cookies[0].name, "auth_token");
        assert!(cookies[0].secure);
        assert!(cookies[0].http_only);
    }

    fn touch_profile(root: &std::path::Path, name: &str, age_secs: u64) {
        let dir = root.join(name);
        fs::create_dir_all(&dir).unwrap();
        let file = fs::File::create(dir.join("cookies.sqlite")).unwrap();
        file.set_modified(std::time::SystemTime::now() - std::time::Duration::from_secs(age_secs))
            .unwrap();
    }

    #[test]
    fn auto_pick_follows_profiles_ini_default() {
        let temp_dir = tempdir().unwrap();
        let root = temp_dir.path().join("Profiles");
        touch_profile(&root, "aaa.default-release", 10);
        touch_profile(&root, "zzz.work", 1000);
        fs::write(
            temp_dir.path().join("profiles.ini"),
            "[Profile0]\nName=work\nIsRelative=1\nPath=Profiles/zzz.work\nDefault=1\n\n[General]\nStartWithLastProfile=1\n",
        )
        .unwrap();
        let picked = pick_default_cookies_db(&root, temp_dir.path()).unwrap();
        assert_eq!(picked, root.join("zzz.work/cookies.sqlite"));
    }

    #[test]
    fn auto_pick_prefers_install_default_over_profile_default() {
        let ini = "[Install4F96D1932A9F858E]\nDefault=Profiles/new.profile\nLocked=1\n\n[Profile0]\nPath=Profiles/old.profile\nDefault=1\n";
        let dir = std::path::Path::new("/ff");
        assert_eq!(
            default_profile_dir_from_ini(ini, dir),
            Some(dir.join("Profiles/new.profile"))
        );
    }

    #[test]
    fn auto_pick_falls_back_to_most_recent_cookie_db() {
        let temp_dir = tempdir().unwrap();
        let root = temp_dir.path().join("Profiles");
        touch_profile(&root, "aaa.default-release", 5000);
        touch_profile(&root, "mmm.used", 10);
        fs::create_dir_all(root.join("zzz.empty")).unwrap();
        let picked = pick_default_cookies_db(&root, temp_dir.path()).unwrap();
        assert_eq!(picked, root.join("mmm.used/cookies.sqlite"));
    }
}
