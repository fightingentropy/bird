use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::PathBuf;
use std::sync::{LazyLock, OnceLock};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use regex::Regex;
use serde::{Deserialize, Serialize};

use crate::features::{features_path, features_snapshot};
use crate::transport::{HttpRequest, HttpTransport};
use crate::types::QueryIdSnapshot;

static BUNDLE_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"https://abs\.twimg\.com/responsive-web/client-web(?:-legacy)?/[A-Za-z0-9.\-]+\.js")
        .expect("valid regex")
});
static QUERY_ID_PATTERNS: LazyLock<Vec<Regex>> = LazyLock::new(|| {
    vec![
        Regex::new(r#"e\.exports=\{queryId\s*:\s*["']([^"']+)["']\s*,\s*operationName\s*:\s*["']([^"']+)["']"#).expect("valid regex"),
        Regex::new(r#"e\.exports=\{operationName\s*:\s*["']([^"']+)["']\s*,\s*queryId\s*:\s*["']([^"']+)["']"#).expect("valid regex"),
        Regex::new(r#"operationName\s*[:=]\s*["']([^"']+)["'](.{0,4000}?)queryId\s*[:=]\s*["']([^"']+)["']"#).expect("valid regex"),
        Regex::new(r#"queryId\s*[:=]\s*["']([^"']+)["'](.{0,4000}?)operationName\s*[:=]\s*["']([^"']+)["']"#).expect("valid regex"),
    ]
});

const DISCOVERY_PAGES: &[&str] = &[
    "https://x.com/?lang=en",
    "https://x.com/explore",
    "https://x.com/notifications",
    "https://x.com/settings/profile",
];
const DEFAULT_TTL: Duration = Duration::from_secs(24 * 60 * 60);

#[derive(Debug, Clone, Serialize, Deserialize)]
struct Snapshot {
    fetched_at: String,
    ttl_ms: u64,
    ids: BTreeMap<String, String>,
    discovery: Discovery,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct Discovery {
    pages: Vec<String>,
    bundles: Vec<String>,
}

pub fn default_cache_path() -> PathBuf {
    std::env::var("BIRD_QUERY_IDS_CACHE")
        .ok()
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            dirs::home_dir()
                .unwrap_or_else(|| PathBuf::from("."))
                .join(".config/bird/query-ids-cache.json")
        })
}

pub fn fallback_query_ids() -> BTreeMap<String, String> {
    BTreeMap::from([
        ("CreateTweet".into(), "TAJw1rBsjAtdNgTdlo2oeg".into()),
        ("CreateRetweet".into(), "ojPdsZsimiJrUGLR1sjUtA".into()),
        ("DeleteRetweet".into(), "iQtK4dl5hBmXewYZuEOKVw".into()),
        ("CreateFriendship".into(), "8h9JVdV8dlSyqyRDJEPCsA".into()),
        ("DestroyFriendship".into(), "ppXWuagMNXgvzx6WoXBW0Q".into()),
        ("FavoriteTweet".into(), "lI07N6Otwv1PhnEgXILM7A".into()),
        ("UnfavoriteTweet".into(), "ZYKSe-w7KEslx3JhSIk5LA".into()),
        ("CreateBookmark".into(), "aoDbu3RHznuiSkQ9aNM67Q".into()),
        ("DeleteBookmark".into(), "Wlmlj2-xzyS1GN3a6cj-mQ".into()),
        ("TweetDetail".into(), "97JF30KziU00483E_8elBA".into()),
        ("SearchTimeline".into(), "M1jEez78PEfVfbQLvlWMvQ".into()),
        ("UserArticlesTweets".into(), "8zBy9h4L90aDL02RsBcCFg".into()),
        ("UserTweets".into(), "Wms1GvIiHXAPBaCr9KblaA".into()),
        ("Bookmarks".into(), "RV1g3b8n_SGOHwkqKYSCFw".into()),
        ("Following".into(), "BEkNpEt5pNETESoqMsTEGA".into()),
        ("Followers".into(), "kuFUYP9eV1FPoEy4N-pi7w".into()),
        ("Likes".into(), "JR2gceKucIKcVNB_9JkhsA".into()),
        ("BookmarkFolderTimeline".into(), "KJIQpsvxrTfRIlbaRIySHQ".into()),
        ("ListOwnerships".into(), "wQcOSjSQ8NtgxIwvYl1lMg".into()),
        ("ListLatestTweetsTimeline".into(), "2TemLyqrMpTeAmysdbnVqw".into()),
        ("ListByRestId".into(), "wXzyA5vM_aVkBL9G8Vp3kw".into()),
        ("HomeTimeline".into(), "edseUwk9sP5Phz__9TIRnA".into()),
        ("HomeLatestTimeline".into(), "iOEZpOdfekFsxSlPQCQtPg".into()),
        ("ExploreSidebar".into(), "lpSN4M6qpimkF4nRFPE3nQ".into()),
        ("ExplorePage".into(), "kheAINB_4pzRDqkzG3K-ng".into()),
        ("GenericTimelineById".into(), "uGSr7alSjR9v6QJAIaqSKQ".into()),
        ("TrendHistory".into(), "Sj4T-jSB9pr0Mxtsc1UKZQ".into()),
        ("AboutAccountQuery".into(), "zs_jFPFT78rBpXv9Z3U2YQ".into()),
    ])
}

pub struct RuntimeQueryIdStore {
    cache_path: PathBuf,
    ttl: Duration,
    cached: OnceLock<Option<Snapshot>>,
}

impl Default for RuntimeQueryIdStore {
    fn default() -> Self {
        Self {
            cache_path: default_cache_path(),
            ttl: DEFAULT_TTL,
            cached: OnceLock::new(),
        }
    }
}

impl RuntimeQueryIdStore {
    pub fn new(cache_path: Option<PathBuf>, ttl: Option<Duration>) -> Self {
        Self {
            cache_path: cache_path.unwrap_or_else(default_cache_path),
            ttl: ttl.unwrap_or(DEFAULT_TTL),
            cached: OnceLock::new(),
        }
    }

    pub fn get_query_id(&self, operation: &str) -> Option<String> {
        self.cached_snapshot()
            .as_ref()
            .and_then(|snapshot| snapshot.ids.get(operation).cloned())
            .or_else(|| fallback_query_ids().get(operation).cloned())
    }

    pub fn snapshot(&self) -> QueryIdSnapshot {
        let features = features_snapshot();
        let features_path = features_path();
        if let Some(snapshot) = self.read_snapshot() {
            let age_ms = snapshot_age_ms(&snapshot);
            QueryIdSnapshot {
                cached: true,
                cache_path: self.cache_path.clone(),
                fetched_at: Some(snapshot.fetched_at.clone()),
                is_fresh: Some(age_ms.map(|age| age <= snapshot.ttl_ms).unwrap_or(false)),
                age_ms,
                ids: snapshot.ids.clone(),
                discovery: serde_json::to_value(&snapshot.discovery).ok(),
                features_path,
                features,
            }
        } else {
            QueryIdSnapshot {
                cached: false,
                cache_path: self.cache_path.clone(),
                fetched_at: None,
                is_fresh: None,
                age_ms: None,
                ids: BTreeMap::new(),
                discovery: None,
                features_path,
                features,
            }
        }
    }

    pub fn refresh(&self, transport: &dyn HttpTransport, operations: &[String]) -> anyhow::Result<QueryIdSnapshot> {
        let bundle_urls = discover_bundles(transport)?;
        let discovered = fetch_and_extract(transport, &bundle_urls, operations)?;
        if discovered.is_empty() {
            return Ok(self.snapshot());
        }
        let snapshot = Snapshot {
            fetched_at: chrono_like_now(),
            ttl_ms: self.ttl.as_millis() as u64,
            ids: discovered,
            discovery: Discovery {
                pages: DISCOVERY_PAGES.iter().map(|page| (*page).to_owned()).collect(),
                bundles: bundle_urls
                    .into_iter()
                    .map(|url| url.rsplit('/').next().unwrap_or(url.as_str()).to_owned())
                    .collect(),
            },
        };
        if let Some(parent) = self.cache_path.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(&self.cache_path, format!("{}\n", serde_json::to_string_pretty(&snapshot)?))?;
        Ok(self.snapshot())
    }

    fn cached_snapshot(&self) -> &Option<Snapshot> {
        self.cached.get_or_init(|| self.read_snapshot())
    }

    fn read_snapshot(&self) -> Option<Snapshot> {
        let raw = fs::read_to_string(&self.cache_path).ok()?;
        serde_json::from_str(&raw).ok()
    }
}

pub fn target_query_id_operations() -> Vec<String> {
    fallback_query_ids().into_keys().collect()
}

fn discover_bundles(transport: &dyn HttpTransport) -> anyhow::Result<Vec<String>> {
    let mut bundles = BTreeSet::new();
    for page in DISCOVERY_PAGES {
        let response = transport.send(&HttpRequest {
            method: "GET".into(),
            url: (*page).to_owned(),
            headers: vec![
                ("User-Agent".into(), "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/129.0.0.0 Safari/537.36".into()),
                ("Accept".into(), "text/html,application/json;q=0.9,*/*;q=0.8".into()),
                ("Accept-Language".into(), "en-US,en;q=0.9".into()),
            ],
            body: None,
            timeout: Some(Duration::from_secs(20)),
        });
        let Ok(response) = response else {
            continue;
        };
        if !response.is_success() {
            continue;
        }
        for capture in BUNDLE_RE.find_iter(&response.text()) {
            bundles.insert(capture.as_str().to_owned());
        }
    }
    if bundles.is_empty() {
        anyhow::bail!("No client bundles discovered; x.com layout may have changed.");
    }
    Ok(bundles.into_iter().collect())
}

fn fetch_and_extract(
    transport: &dyn HttpTransport,
    bundle_urls: &[String],
    operations: &[String],
) -> anyhow::Result<BTreeMap<String, String>> {
    let targets = operations.iter().cloned().collect::<BTreeSet<_>>();
    let mut discovered = BTreeMap::new();
    for bundle_url in bundle_urls {
        if discovered.len() == targets.len() {
            break;
        }
        let response = transport.send(&HttpRequest {
            method: "GET".into(),
            url: bundle_url.clone(),
            headers: vec![],
            body: None,
            timeout: Some(Duration::from_secs(20)),
        })?;
        if !response.is_success() {
            continue;
        }
        let js = response.text();
        for (operation_name, query_id) in extract_operations(&js, &QUERY_ID_PATTERNS) {
            if targets.contains(&operation_name) && !discovered.contains_key(&operation_name) {
                discovered.insert(operation_name, query_id);
            }
        }
    }
    Ok(discovered)
}

fn extract_operations(js: &str, patterns: &[Regex]) -> Vec<(String, String)> {
    let mut operations = Vec::new();
    for (index, pattern) in patterns.iter().enumerate() {
        for capture in pattern.captures_iter(js) {
            let (operation_name, query_id) = match index {
                0 => (capture.get(2), capture.get(1)),
                1 => (capture.get(1), capture.get(2)),
                2 => (capture.get(1), capture.get(3)),
                3 => (capture.get(3), capture.get(1)),
                _ => unreachable!(),
            };
            let Some(operation_name) = operation_name.map(|value| value.as_str().to_owned()) else {
                continue;
            };
            let Some(query_id) = query_id.map(|value| value.as_str().to_owned()) else {
                continue;
            };
            operations.push((operation_name, query_id));
        }
    }
    operations
}

fn snapshot_age_ms(snapshot: &Snapshot) -> Option<u64> {
    let fetched_at = chrono_like_parse(&snapshot.fetched_at)?;
    let now = SystemTime::now().duration_since(UNIX_EPOCH).ok()?.as_millis() as u64;
    Some(now.saturating_sub(fetched_at))
}

fn chrono_like_now() -> String {
    let now = chrono_like_timestamp();
    format!(
        "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}.000Z",
        now.0, now.1, now.2, now.3, now.4, now.5
    )
}

fn chrono_like_parse(value: &str) -> Option<u64> {
    let parsed = time_like::parse_rfc3339_millis(value)?;
    Some(parsed)
}

fn chrono_like_timestamp() -> (u64, u64, u64, u64, u64, u64) {
    time_like::utc_components(
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|duration| duration.as_secs() as u64)
            .unwrap_or_default(),
    )
}

mod time_like {
    pub fn parse_rfc3339_millis(value: &str) -> Option<u64> {
        let date_time = value.split('T').collect::<Vec<_>>();
        if date_time.len() != 2 {
            return None;
        }
        let date = date_time[0].split('-').collect::<Vec<_>>();
        let time = date_time[1].trim_end_matches('Z').split(':').collect::<Vec<_>>();
        if date.len() != 3 || time.len() != 3 {
            return None;
        }
        let year = date[0].parse::<i32>().ok()?;
        let month = date[1].parse::<u32>().ok()?;
        let day = date[2].parse::<u32>().ok()?;
        let second_parts = time[2].split('.').collect::<Vec<_>>();
        let hour = time[0].parse::<u32>().ok()?;
        let minute = time[1].parse::<u32>().ok()?;
        let second = second_parts.first()?.parse::<u32>().ok()?;
        let days = days_since_unix_epoch(year, month, day)?;
        Some(
            ((days * 24 + hour as u64) * 60 + minute as u64) * 60_000
                + second as u64 * 1_000,
        )
    }

    pub fn utc_components(mut seconds: u64) -> (u64, u64, u64, u64, u64, u64) {
        let second = seconds % 60;
        seconds /= 60;
        let minute = seconds % 60;
        seconds /= 60;
        let hour = seconds % 24;
        let days = seconds / 24;
        let (year, month, day) = civil_from_days(days as i64);
        (year as u64, month as u64, day as u64, hour, minute, second)
    }

    fn days_since_unix_epoch(year: i32, month: u32, day: u32) -> Option<u64> {
        let days = days_from_civil(year as i64, month as i64, day as i64);
        if days < 0 {
            None
        } else {
            Some(days as u64)
        }
    }

    fn days_from_civil(year: i64, month: i64, day: i64) -> i64 {
        let year = year - (month <= 2) as i64;
        let era = if year >= 0 { year } else { year - 399 } / 400;
        let year_of_era = year - era * 400;
        let month = month + if month > 2 { -3 } else { 9 };
        let day_of_year = (153 * month + 2) / 5 + day - 1;
        let day_of_era = year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year;
        era * 146097 + day_of_era - 719468
    }

    fn civil_from_days(days: i64) -> (i64, i64, i64) {
        let days = days + 719468;
        let era = if days >= 0 { days } else { days - 146096 } / 146097;
        let day_of_era = days - era * 146097;
        let year_of_era =
            (day_of_era - day_of_era / 1460 + day_of_era / 36524 - day_of_era / 146096) / 365;
        let mut year = year_of_era + era * 400;
        let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
        let month_piece = (5 * day_of_year + 2) / 153;
        let day = day_of_year - (153 * month_piece + 2) / 5 + 1;
        let month = month_piece + if month_piece < 10 { 3 } else { -9 };
        year += (month <= 2) as i64;
        (year, month, day)
    }
}

#[cfg(test)]
mod tests {
    use regex::Regex;

    use super::extract_operations;

    #[test]
    fn extracts_query_ids_from_bundle_patterns() {
        let patterns = vec![
            Regex::new(r#"e\.exports=\{queryId\s*:\s*["']([^"']+)["']\s*,\s*operationName\s*:\s*["']([^"']+)["']"#)
                .expect("regex"),
            Regex::new(r#"e\.exports=\{operationName\s*:\s*["']([^"']+)["']\s*,\s*queryId\s*:\s*["']([^"']+)["']"#)
                .expect("regex"),
            Regex::new(r#"operationName\s*[:=]\s*["']([^"']+)["'](.{0,4000}?)queryId\s*[:=]\s*["']([^"']+)["']"#)
                .expect("regex"),
            Regex::new(r#"queryId\s*[:=]\s*["']([^"']+)["'](.{0,4000}?)operationName\s*[:=]\s*["']([^"']+)["']"#)
                .expect("regex"),
        ];
        let js = r#"
            e.exports={queryId:"abc123",operationName:"HomeTimeline"}
            const value = { operationName: "TweetDetail", queryId: "def456" };
        "#;

        let operations = extract_operations(js, &patterns);

        assert!(operations.contains(&("HomeTimeline".to_owned(), "abc123".to_owned())));
        assert!(operations.contains(&("TweetDetail".to_owned(), "def456".to_owned())));
    }
}
