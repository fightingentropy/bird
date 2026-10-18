use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::PathBuf;
use std::sync::{LazyLock, Mutex, OnceLock, RwLock};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use regex::Regex;
use serde::{Deserialize, Serialize};

use crate::features::{features_path, features_snapshot};
use crate::transport::{HttpRequest, HttpTransport};
use crate::types::QueryIdSnapshot;

static BUNDLE_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"https://abs\.twimg\.com/responsive-web/client-web(?:-legacy)?/[A-Za-z0-9.\-]+\.js")
        .expect("valid regex")
});
// Chunk ids are JS number literals, so 9000 is written `9e3`; match those whole.
static CHUNK_NAME_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"(\d+(?:e\d+)?):"([^"]*bundle\.Bookmarks[^"]*)""#).expect("valid regex")
});
/// Every named webpack chunk (`12345:"shared~bundle.Foo~bundle.Bar"`), for lazy discovery.
static ANY_CHUNK_NAME_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"(\d+(?:e\d+)?):"([A-Za-z0-9_.~/-]*(?:bundle|loader)\.[A-Za-z0-9_.~/-]+)""#)
        .expect("valid regex")
});
static CHUNK_HASH_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"(\d+(?:e\d+)?):"([a-f0-9]{7,})""#).expect("valid regex"));
static CHUNK_SUFFIX_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"\}\)\[e\]\+"([^"]*\.js)""#).expect("valid regex"));
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
    "https://x.com/i/bookmarks",
    "https://x.com/explore",
    "https://x.com/notifications",
    "https://x.com/settings/profile",
];
/// Signed in, x.com still serves the web client whose bundles carry the query ids;
/// logged-out pages now serve a different client.
const SIGNED_IN_DISCOVERY_PAGE: &str = "https://x.com/home";
const DEFAULT_TTL: Duration = Duration::from_secs(24 * 60 * 60);
/// Lazily loaded chunks scanned at most per refresh, for operations the entry bundles lack.
const MAX_LAZY_CHUNKS: usize = 600;
/// Timeout for a page or entry bundle request.
const REQUEST_TIMEOUT: Duration = Duration::from_secs(20);
/// Lazy chunks are small CDN files; a slower response means a stalled network.
const LAZY_CHUNK_TIMEOUT: Duration = Duration::from_secs(10);
/// The lazy scan gives up after this many chunk requests in a row fail to connect.
const MAX_LAZY_ERRORS: usize = 3;
/// Upper bound for one whole discovery, so a stalled network cannot hold the refresh
/// lock (and every caller waiting on it) for long.
const DISCOVERY_BUDGET: Duration = Duration::from_secs(60);
/// A successful refresh answers matching repeat requests for this long.
const REFRESH_COOLDOWN: Duration = Duration::from_secs(60);
/// A failed refresh answers matching repeat requests with the same error for this long.
const FAILURE_COOLDOWN: Duration = Duration::from_secs(30);
/// Operations X's current web client no longer ships. They are still recorded when a
/// scanned bundle happens to carry them, but the lazy chunk scan does not keep going on
/// their account.
const RETIRED_OPERATIONS: &[&str] = &["AboutAccountQuery", "CreateFriendship", "DestroyFriendship"];

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
        ("Bookmarks".into(), "xtj3H29MLXk0r_3TIgdU5g".into()),
        ("Following".into(), "BEkNpEt5pNETESoqMsTEGA".into()),
        ("Followers".into(), "kuFUYP9eV1FPoEy4N-pi7w".into()),
        ("Likes".into(), "JR2gceKucIKcVNB_9JkhsA".into()),
        (
            "BookmarkFolderTimeline".into(),
            "KJIQpsvxrTfRIlbaRIySHQ".into(),
        ),
        ("ListOwnerships".into(), "wQcOSjSQ8NtgxIwvYl1lMg".into()),
        (
            "ListLatestTweetsTimeline".into(),
            "2TemLyqrMpTeAmysdbnVqw".into(),
        ),
        ("ListByRestId".into(), "wXzyA5vM_aVkBL9G8Vp3kw".into()),
        ("HomeTimeline".into(), "edseUwk9sP5Phz__9TIRnA".into()),
        ("HomeLatestTimeline".into(), "iOEZpOdfekFsxSlPQCQtPg".into()),
        ("ExploreSidebar".into(), "lpSN4M6qpimkF4nRFPE3nQ".into()),
        ("ExplorePage".into(), "kheAINB_4pzRDqkzG3K-ng".into()),
        (
            "GenericTimelineById".into(),
            "uGSr7alSjR9v6QJAIaqSKQ".into(),
        ),
        ("TrendHistory".into(), "Sj4T-jSB9pr0Mxtsc1UKZQ".into()),
        ("AboutAccountQuery".into(), "zs_jFPFT78rBpXv9Z3U2YQ".into()),
    ])
}

pub struct RuntimeQueryIdStore {
    cache_path: PathBuf,
    ttl: Duration,
    cached: OnceLock<RwLock<Option<Snapshot>>>,
    last_attempt: Mutex<Option<RefreshAttempt>>,
}

/// What the last discovery looked for and found, so a repeat request it already covers
/// is answered without going back to the network.
struct RefreshAttempt {
    at: Instant,
    operations: BTreeSet<String>,
    signed_in: bool,
    /// Operations the discovery found.
    found: BTreeSet<String>,
    /// False when a required refresh stopped scanning before every expected operation was
    /// found, so the rest were not searched for.
    exhaustive: bool,
    /// Why it failed, if it did.
    error: Option<String>,
}

impl RefreshAttempt {
    fn new(operations: &[String], signed_in: bool) -> Self {
        Self {
            at: Instant::now(),
            operations: operations.iter().cloned().collect(),
            signed_in,
            found: BTreeSet::new(),
            exhaustive: true,
            error: None,
        }
    }

    /// Whether this earlier attempt answers a new request, so repeating the discovery
    /// would only repeat its outcome: the request asks for nothing the attempt did not
    /// search for, and does not ask for a signed-in discovery the attempt was not.
    fn answers(&self, request: &Self, required_operation: Option<&str>) -> bool {
        let window = if self.error.is_some() {
            FAILURE_COOLDOWN
        } else {
            REFRESH_COOLDOWN
        };
        if self.at.elapsed() >= window || (request.signed_in && !self.signed_in) {
            return false;
        }
        let covers = request.operations.is_subset(&self.operations);
        match (required_operation, self.error.is_some()) {
            (Some(operation), false) => self.found.contains(operation),
            (Some(operation), true) => covers && !self.found.contains(operation),
            (None, false) => covers && self.exhaustive,
            (None, true) => covers && self.found.is_empty(),
        }
    }
}

/// Time left for one discovery.
#[derive(Clone, Copy)]
struct Deadline(Instant);

impl Deadline {
    fn after(budget: Duration) -> Self {
        Self(Instant::now() + budget)
    }

    /// Timeout for the next request: `cap`, or what is left of the budget if that is
    /// less. `None` once the budget is spent.
    fn timeout(self, cap: Duration) -> Option<Duration> {
        let left = self.0.saturating_duration_since(Instant::now());
        (!left.is_zero()).then(|| left.min(cap))
    }
}

impl Default for RuntimeQueryIdStore {
    fn default() -> Self {
        Self::new(None, None)
    }
}

impl RuntimeQueryIdStore {
    pub fn new(cache_path: Option<PathBuf>, ttl: Option<Duration>) -> Self {
        Self {
            cache_path: cache_path.unwrap_or_else(default_cache_path),
            ttl: ttl.unwrap_or(DEFAULT_TTL),
            cached: OnceLock::new(),
            last_attempt: Mutex::new(None),
        }
    }

    pub fn get_query_id(&self, operation: &str) -> Option<String> {
        let cached_id = self
            .cached_snapshot()
            .read()
            .unwrap_or_else(|error| error.into_inner())
            .as_ref()
            .and_then(|snapshot| snapshot.ids.get(operation).cloned());
        cached_id.or_else(|| fallback_query_ids().get(operation).cloned())
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

    pub fn refresh(
        &self,
        transport: &dyn HttpTransport,
        operations: &[String],
    ) -> anyhow::Result<QueryIdSnapshot> {
        self.refresh_inner(transport, operations, None, None)
    }

    /// Like [`refresh`](Self::refresh), discovering from the signed-in web client when
    /// `cookie_header` is given (logged-out pages no longer carry the query ids).
    pub fn refresh_with_cookies(
        &self,
        transport: &dyn HttpTransport,
        operations: &[String],
        cookie_header: Option<&str>,
    ) -> anyhow::Result<QueryIdSnapshot> {
        self.refresh_inner(transport, operations, None, cookie_header)
    }

    pub fn refresh_required(
        &self,
        transport: &dyn HttpTransport,
        operations: &[String],
        required_operation: &str,
    ) -> anyhow::Result<QueryIdSnapshot> {
        self.refresh_inner(transport, operations, Some(required_operation), None)
    }

    pub fn refresh_required_with_cookies(
        &self,
        transport: &dyn HttpTransport,
        operations: &[String],
        required_operation: &str,
        cookie_header: Option<&str>,
    ) -> anyhow::Result<QueryIdSnapshot> {
        self.refresh_inner(
            transport,
            operations,
            Some(required_operation),
            cookie_header,
        )
    }

    fn refresh_inner(
        &self,
        transport: &dyn HttpTransport,
        operations: &[String],
        required_operation: Option<&str>,
        cookie_header: Option<&str>,
    ) -> anyhow::Result<QueryIdSnapshot> {
        let mut attempt = RefreshAttempt::new(operations, cookie_header.is_some());
        // Held for the whole discovery: callers that queue behind it get its outcome.
        let mut last_attempt = self
            .last_attempt
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        if let Some(previous) = last_attempt
            .as_ref()
            .filter(|previous| previous.answers(&attempt, required_operation))
        {
            return match &previous.error {
                Some(error) => Err(anyhow::anyhow!(
                    "{error} (query id discovery failed recently; not retrying yet)"
                )),
                None => Ok(self.snapshot()),
            };
        }
        let result = self.discover_and_store(
            transport,
            operations,
            required_operation,
            cookie_header,
            &mut attempt,
        );
        attempt.at = Instant::now();
        attempt.error = result.as_ref().err().map(|error| format!("{error:#}"));
        *last_attempt = Some(attempt);
        result
    }

    fn discover_and_store(
        &self,
        transport: &dyn HttpTransport,
        operations: &[String],
        required_operation: Option<&str>,
        cookie_header: Option<&str>,
        attempt: &mut RefreshAttempt,
    ) -> anyhow::Result<QueryIdSnapshot> {
        let deadline = Deadline::after(DISCOVERY_BUDGET);
        let bundles = discover_bundles(transport, cookie_header, deadline)?;
        let Extraction {
            ids: discovered,
            lazy_used,
            exhaustive,
        } = fetch_and_extract(
            transport,
            &bundles,
            operations,
            required_operation,
            deadline,
        )?;
        attempt.found = discovered.keys().cloned().collect();
        attempt.exhaustive = exhaustive;
        if discovered.is_empty() {
            anyhow::bail!("No GraphQL query IDs discovered; refusing to reuse a stale cache.");
        }
        if let Some(required_operation) = required_operation
            && !discovered.contains_key(required_operation)
        {
            anyhow::bail!(
                "Could not discover the current {required_operation} query; refusing to reuse a stale cache."
            );
        }
        let mut ids = self
            .read_snapshot()
            .map(|snapshot| snapshot.ids)
            .unwrap_or_default();
        ids.extend(discovered);
        let snapshot = Snapshot {
            fetched_at: chrono_like_now(),
            ttl_ms: self.ttl.as_millis() as u64,
            ids,
            discovery: Discovery {
                pages: bundles.pages,
                bundles: bundles
                    .entry
                    .iter()
                    .chain(&lazy_used)
                    .map(|url| url.rsplit('/').next().unwrap_or(url.as_str()).to_owned())
                    .collect(),
            },
        };
        if let Some(parent) = self.cache_path.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(
            &self.cache_path,
            format!("{}\n", serde_json::to_string_pretty(&snapshot)?),
        )?;
        *self
            .cached_snapshot()
            .write()
            .unwrap_or_else(|error| error.into_inner()) = Some(snapshot);
        Ok(self.snapshot())
    }

    fn cached_snapshot(&self) -> &RwLock<Option<Snapshot>> {
        self.cached
            .get_or_init(|| RwLock::new(self.read_snapshot()))
    }

    fn read_snapshot(&self) -> Option<Snapshot> {
        let raw = fs::read_to_string(&self.cache_path).ok()?;
        serde_json::from_str(&raw).ok()
    }
}

pub fn target_query_id_operations() -> Vec<String> {
    fallback_query_ids().into_keys().collect()
}

/// Bundles to scan: `entry` (the page's scripts plus known lazy bookmark bundles) first,
/// then `lazy` (every other named chunk, most likely first) for operations still missing.
#[derive(Debug, Default)]
struct DiscoveredBundles {
    pages: Vec<String>,
    entry: Vec<String>,
    lazy: Vec<String>,
}

fn discover_bundles(
    transport: &dyn HttpTransport,
    cookie_header: Option<&str>,
    deadline: Deadline,
) -> anyhow::Result<DiscoveredBundles> {
    let mut pages = Vec::new();
    let mut entry = BTreeSet::new();
    let mut lazy = BTreeSet::new();
    // Records a loaded page; true once any bundle has been found.
    let mut absorb = |url: &str, body: String| {
        pages.push(url.to_owned());
        for capture in BUNDLE_RE.find_iter(&body) {
            entry.insert(capture.as_str().to_owned());
        }
        entry.extend(discover_lazy_bookmark_bundles(&body));
        lazy.extend(discover_lazy_chunks(&body));
        !entry.is_empty()
    };

    let signed_in = cookie_header
        .and_then(|_| {
            fetch_discovery_page(transport, SIGNED_IN_DISCOVERY_PAGE, cookie_header, deadline)
        })
        .is_some_and(|body| absorb(SIGNED_IN_DISCOVERY_PAGE, body));
    if !signed_in {
        for page in DISCOVERY_PAGES {
            if let Some(body) = fetch_discovery_page(transport, page, None, deadline)
                && !absorb(page, body)
            {
                // A logged-out page that loads without any bundle is the new client,
                // which carries none; the other pages would only redirect to login.
                break;
            }
        }
    }
    if entry.is_empty() && lazy.is_empty() {
        if cookie_header.is_none() {
            anyhow::bail!(
                "No client bundles discovered; x.com only serves them to a signed-in session, or its layout changed."
            );
        }
        anyhow::bail!("No client bundles discovered; x.com layout may have changed.");
    }
    Ok(DiscoveredBundles {
        pages,
        lazy: prioritize_lazy_chunks(lazy.difference(&entry).cloned().collect()),
        entry: entry.into_iter().collect(),
    })
}

fn fetch_discovery_page(
    transport: &dyn HttpTransport,
    url: &str,
    cookie_header: Option<&str>,
    deadline: Deadline,
) -> Option<String> {
    let timeout = deadline.timeout(REQUEST_TIMEOUT)?;
    let mut headers = vec![
        ("User-Agent".into(), "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/129.0.0.0 Safari/537.36".into()),
        ("Accept".into(), "text/html,application/json;q=0.9,*/*;q=0.8".into()),
        ("Accept-Language".into(), "en-US,en;q=0.9".into()),
    ];
    if let Some(cookie_header) = cookie_header {
        headers.push(("Cookie".into(), cookie_header.to_owned()));
    }
    let response = transport
        .send(&HttpRequest {
            method: "GET".into(),
            url: url.to_owned(),
            headers,
            body: None,
            timeout: Some(timeout),
        })
        .ok()?;
    response.is_success().then(|| response.text())
}

/// Every named chunk in the page's webpack chunk map, as a URL.
fn discover_lazy_chunks(html: &str) -> Vec<String> {
    let Some(base_url) = BUNDLE_RE
        .find(html)
        .and_then(|url| url.as_str().rsplit_once('/').map(|(base, _)| base))
    else {
        return Vec::new();
    };
    let suffix = CHUNK_SUFFIX_RE
        .captures(html)
        .and_then(|capture| capture.get(1))
        .map_or("a.js", |value| value.as_str());
    let hashes = chunk_hashes(html);
    ANY_CHUNK_NAME_RE
        .captures_iter(html)
        .filter_map(|capture| {
            let name = capture.get(2)?.as_str();
            let hash = hashes.get(capture.get(1)?.as_str())?;
            Some(format!("{base_url}/{name}.{hash}{suffix}"))
        })
        .collect()
}

/// Chunk id to content hash, from the page's webpack hash map.
fn chunk_hashes(html: &str) -> BTreeMap<&str, &str> {
    CHUNK_HASH_RE
        .captures_iter(html)
        .filter_map(|capture| Some((capture.get(1)?.as_str(), capture.get(2)?.as_str())))
        .collect()
}

/// Bookmark and list chunks first (where bird's lazily loaded operations live), then
/// shared chunks, then the rest.
fn prioritize_lazy_chunks(mut urls: Vec<String>) -> Vec<String> {
    let rank = |url: &str| {
        let name = url.rsplit('/').next().unwrap_or(url);
        if name.contains("Bookmark") || name.contains("List") {
            0
        } else if name.starts_with("shared~") {
            1
        } else {
            2
        }
    };
    urls.sort_by(|a, b| rank(a).cmp(&rank(b)).then_with(|| a.cmp(b)));
    urls
}

fn discover_lazy_bookmark_bundles(html: &str) -> Vec<String> {
    let Some(base_url) = BUNDLE_RE
        .find(html)
        .and_then(|url| url.as_str().rsplit_once('/').map(|(base, _)| base))
    else {
        return Vec::new();
    };
    let Some(suffix) = CHUNK_SUFFIX_RE
        .captures(html)
        .and_then(|capture| capture.get(1))
        .map(|value| value.as_str())
    else {
        return Vec::new();
    };
    let hashes = chunk_hashes(html);

    CHUNK_NAME_RE
        .captures_iter(html)
        .filter_map(|capture| {
            let chunk_id = capture.get(1)?.as_str();
            let name = capture.get(2)?.as_str();
            let hash = hashes.get(chunk_id)?;
            Some(format!("{base_url}/{name}.{hash}{suffix}"))
        })
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect()
}

/// What a scan of the bundles found.
struct Extraction {
    ids: BTreeMap<String, String>,
    /// Lazy chunks that contributed any of the ids.
    lazy_used: Vec<String>,
    /// False when a required refresh stopped before every expected operation was found.
    exhaustive: bool,
}

/// Scan the entry bundles, then lazy chunks until every expected operation is found, or,
/// for a required refresh, until the required one is. The lazy scan also stops when the
/// budget runs out or the network stops answering.
fn fetch_and_extract(
    transport: &dyn HttpTransport,
    bundles: &DiscoveredBundles,
    operations: &[String],
    required_operation: Option<&str>,
    deadline: Deadline,
) -> anyhow::Result<Extraction> {
    let targets = operations.iter().cloned().collect::<BTreeSet<_>>();
    let expected = targets
        .iter()
        .filter(|operation| !RETIRED_OPERATIONS.contains(&operation.as_str()))
        .collect::<Vec<_>>();
    let mut discovered = BTreeMap::new();
    let mut lazy_used = Vec::new();
    for bundle_url in &bundles.entry {
        if discovered.len() == targets.len() {
            break;
        }
        let Some(timeout) = deadline.timeout(REQUEST_TIMEOUT) else {
            break;
        };
        let response = transport.send(&HttpRequest {
            method: "GET".into(),
            url: bundle_url.clone(),
            headers: vec![],
            body: None,
            timeout: Some(timeout),
        })?;
        if !response.is_success() {
            continue;
        }
        extract_targets(&response.text(), &targets, &mut discovered);
    }
    let mut exhaustive = true;
    let mut connect_errors = 0;
    for chunk_url in bundles.lazy.iter().take(MAX_LAZY_CHUNKS) {
        if expected
            .iter()
            .all(|operation| discovered.contains_key(*operation))
        {
            break;
        }
        if required_operation.is_some_and(|operation| discovered.contains_key(operation)) {
            exhaustive = false;
            break;
        }
        let Some(timeout) = deadline.timeout(LAZY_CHUNK_TIMEOUT) else {
            break;
        };
        let Ok(response) = transport.send(&HttpRequest {
            method: "GET".into(),
            url: chunk_url.clone(),
            headers: vec![],
            body: None,
            timeout: Some(timeout),
        }) else {
            connect_errors += 1;
            if connect_errors >= MAX_LAZY_ERRORS {
                break;
            }
            continue;
        };
        connect_errors = 0;
        if !response.is_success() {
            continue;
        }
        if extract_targets(&response.text(), &targets, &mut discovered) {
            lazy_used.push(chunk_url.clone());
        }
    }
    Ok(Extraction {
        ids: discovered,
        lazy_used,
        exhaustive,
    })
}

/// Record target operations found in `js`; true if any were new.
fn extract_targets(
    js: &str,
    targets: &BTreeSet<String>,
    discovered: &mut BTreeMap<String, String>,
) -> bool {
    let mut added = false;
    for (operation_name, query_id) in extract_operations(js, &QUERY_ID_PATTERNS) {
        if targets.contains(&operation_name) && !discovered.contains_key(&operation_name) {
            discovered.insert(operation_name, query_id);
            added = true;
        }
    }
    added
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
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .ok()?
        .as_millis() as u64;
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
            .map(|duration| duration.as_secs())
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
        let time = date_time[1]
            .trim_end_matches('Z')
            .split(':')
            .collect::<Vec<_>>();
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
        Some(((days * 24 + hour as u64) * 60 + minute as u64) * 60_000 + second as u64 * 1_000)
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
        if days < 0 { None } else { Some(days as u64) }
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
    use std::collections::BTreeMap;
    use std::fs;
    use std::time::Duration;

    use regex::Regex;

    use crate::transport::{HttpRequest, HttpResponse, HttpTransport};

    use std::sync::Mutex;
    use std::sync::atomic::{AtomicUsize, Ordering};

    use super::{
        Deadline, DiscoveredBundles, Discovery, MAX_LAZY_ERRORS, RuntimeQueryIdStore,
        SIGNED_IN_DISCOVERY_PAGE, Snapshot, chunk_hashes, discover_lazy_bookmark_bundles,
        discover_lazy_chunks, extract_operations, fetch_and_extract, prioritize_lazy_chunks,
    };

    /// Logged out, x.com serves the new client (no bundles); signed in, the web client
    /// with a main bundle and a lazily loaded list chunk.
    #[derive(Default)]
    struct SignedInTransport {
        anonymous_pages: AtomicUsize,
        requests: AtomicUsize,
    }

    impl HttpTransport for SignedInTransport {
        fn send(&self, request: &HttpRequest) -> anyhow::Result<HttpResponse> {
            self.requests.fetch_add(1, Ordering::SeqCst);
            let signed_in = request.headers.iter().any(|(name, value)| {
                name.eq_ignore_ascii_case("cookie") && value.contains("auth_token")
            });
            let body = if request.url.ends_with("/main.abcdef01.js") {
                br#"e.exports={queryId:"home-id",operationName:"HomeTimeline"}"#.to_vec()
            } else if request
                .url
                .ends_with("/shared~bundle.UserLists.1234567a.js")
            {
                br#"e.exports={queryId:"lists-id",operationName:"ListOwnerships"}"#.to_vec()
            } else if request.url.ends_with(".js") {
                Vec::new()
            } else if request.url == SIGNED_IN_DISCOVERY_PAGE && signed_in {
                br#"
                    <script src="https://abs.twimg.com/responsive-web/client-web/main.abcdef01.js"></script>
                    <script>t.u=e=>(({101:"bundle.Grok",202:"shared~bundle.UserLists"})[e]||e)
                        +"."+({101:"7654321",202:"1234567"})[e]+"a.js"</script>
                "#
                .to_vec()
            } else {
                self.anonymous_pages.fetch_add(1, Ordering::SeqCst);
                br#"<script type="module" src="https://abs.twimg.com/x-web/x-web/entry-client-logged-out.js"></script>"#
                    .to_vec()
            };
            Ok(HttpResponse {
                status: 200,
                body,
                ..HttpResponse::default()
            })
        }
    }

    #[test]
    fn discovers_from_the_signed_in_page_and_lazy_chunks() {
        let temp = tempfile::tempdir().expect("temp directory");
        let store = RuntimeQueryIdStore::new(
            Some(temp.path().join("query-ids.json")),
            Some(Duration::from_secs(60)),
        );
        let transport = SignedInTransport::default();
        let snapshot = store
            .refresh_with_cookies(
                &transport,
                &["HomeTimeline".to_owned(), "ListOwnerships".to_owned()],
                Some("auth_token=a; ct0=b"),
            )
            .expect("refresh");

        assert_eq!(
            snapshot.ids.get("HomeTimeline").map(String::as_str),
            Some("home-id")
        );
        assert_eq!(
            snapshot.ids.get("ListOwnerships").map(String::as_str),
            Some("lists-id")
        );
        assert_eq!(transport.anonymous_pages.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn repeated_plain_refreshes_reuse_the_last_result() {
        let temp = tempfile::tempdir().expect("temp directory");
        let store = RuntimeQueryIdStore::new(
            Some(temp.path().join("query-ids.json")),
            Some(Duration::from_secs(60)),
        );
        let transport = SignedInTransport::default();
        let operations = ["HomeTimeline".to_owned()];
        store
            .refresh_with_cookies(&transport, &operations, Some("auth_token=a; ct0=b"))
            .expect("first refresh");
        let after_first = transport.requests.load(Ordering::SeqCst);
        store
            .refresh_with_cookies(&transport, &operations, Some("auth_token=a; ct0=b"))
            .expect("second refresh");
        assert_eq!(transport.requests.load(Ordering::SeqCst), after_first);
    }

    #[test]
    fn lists_every_named_chunk_and_ranks_likely_ones_first() {
        let html = r#"<script src="https://abs.twimg.com/responsive-web/client-web/main.abcdef01.js"></script>
            t.u=e=>(({1:"bundle.Grok",2:"shared~bundle.UserLists",3:"shared~bundle.AudioSpace",4:"i18n/en"})[e]||e)
            +"."+({1:"aaaaaaa",2:"bbbbbbb",3:"ccccccc",4:"ddddddd"})[e]+"a.js""#;
        let urls = prioritize_lazy_chunks(discover_lazy_chunks(html));
        let names = urls
            .iter()
            .map(|url| url.rsplit('/').next().unwrap_or_default())
            .collect::<Vec<_>>();
        assert_eq!(
            names,
            vec![
                "shared~bundle.UserLists.bbbbbbba.js",
                "shared~bundle.AudioSpace.ccccccca.js",
                "bundle.Grok.aaaaaaaa.js",
            ]
        );
    }

    const COOKIE: Option<&str> = Some("auth_token=a; ct0=b");

    /// A signed-in page listing `chunks` lazily loaded chunks, `shared~bundle.Chunk000`
    /// onwards. `main.abcdef01.js` carries HomeTimeline; `bodies` gives other files' source.
    struct ChunkTransport {
        chunks: usize,
        bodies: BTreeMap<&'static str, &'static str>,
        chunk_errors: bool,
        requests: Mutex<Vec<String>>,
    }

    impl ChunkTransport {
        fn new(chunks: usize, bodies: &[(&'static str, &'static str)]) -> Self {
            Self {
                chunks,
                bodies: bodies.iter().copied().collect(),
                chunk_errors: false,
                requests: Mutex::new(Vec::new()),
            }
        }

        fn chunk_requests(&self) -> usize {
            self.requests
                .lock()
                .expect("requests")
                .iter()
                .filter(|url| url.contains("shared~bundle.Chunk"))
                .count()
        }
    }

    impl HttpTransport for ChunkTransport {
        fn send(&self, request: &HttpRequest) -> anyhow::Result<HttpResponse> {
            self.requests
                .lock()
                .expect("requests")
                .push(request.url.clone());
            if self.chunk_errors && request.url.contains("shared~bundle.Chunk") {
                anyhow::bail!("connection timed out");
            }
            let body = if request.url == SIGNED_IN_DISCOVERY_PAGE {
                let names = (0..self.chunks)
                    .map(|index| format!(r#"{}:"shared~bundle.Chunk{index:03}""#, 1000 + index))
                    .collect::<Vec<_>>()
                    .join(",");
                let hashes = (0..self.chunks)
                    .map(|index| format!(r#"{}:"{index:07x}""#, 1000 + index))
                    .collect::<Vec<_>>()
                    .join(",");
                format!(
                    r#"<script src="https://abs.twimg.com/responsive-web/client-web/main.abcdef01.js"></script>
                    <script>t.u=e=>(({{{names}}})[e]||e)+"."+({{{hashes}}})[e]+"a.js"</script>"#
                )
                .into_bytes()
            } else if request.url.ends_with("/main.abcdef01.js") {
                br#"e.exports={queryId:"home-id",operationName:"HomeTimeline"}"#.to_vec()
            } else {
                self.bodies
                    .iter()
                    .find(|(suffix, _)| request.url.contains(*suffix))
                    .map(|(_, body)| body.as_bytes().to_vec())
                    .unwrap_or_default()
            };
            Ok(HttpResponse {
                status: 200,
                body,
                ..HttpResponse::default()
            })
        }
    }

    const LISTS_BODY: &str = r#"e.exports={queryId:"lists-id",operationName:"ListOwnerships"}"#;

    fn temp_store(temp: &tempfile::TempDir) -> RuntimeQueryIdStore {
        RuntimeQueryIdStore::new(
            Some(temp.path().join("query-ids.json")),
            Some(Duration::from_secs(60)),
        )
    }

    fn names(operations: &[&str]) -> Vec<String> {
        operations.iter().map(|name| (*name).to_owned()).collect()
    }

    #[test]
    fn lazy_scan_stops_once_the_expected_operations_are_found() {
        let temp = tempfile::tempdir().expect("temp directory");
        let transport = ChunkTransport::new(40, &[("Chunk003", LISTS_BODY)]);
        // AboutAccountQuery is retired: its absence must not keep the scan going.
        let snapshot = temp_store(&temp)
            .refresh_with_cookies(
                &transport,
                &names(&["HomeTimeline", "ListOwnerships", "AboutAccountQuery"]),
                COOKIE,
            )
            .expect("refresh");

        assert_eq!(snapshot.ids.len(), 2);
        assert_eq!(transport.chunk_requests(), 4);
    }

    #[test]
    fn required_refresh_skips_the_lazy_scan_when_the_entry_bundles_have_it() {
        let temp = tempfile::tempdir().expect("temp directory");
        let store = temp_store(&temp);
        let transport = ChunkTransport::new(40, &[("Chunk003", LISTS_BODY)]);
        let operations = names(&["HomeTimeline", "ListOwnerships"]);

        store
            .refresh_required_with_cookies(&transport, &operations, "HomeTimeline", COOKIE)
            .expect("required refresh");
        assert_eq!(transport.chunk_requests(), 0);

        // It did not search for the rest, so a plain refresh is not answered by it ...
        let snapshot = store
            .refresh_with_cookies(&transport, &operations, COOKIE)
            .expect("plain refresh");
        assert_eq!(transport.chunk_requests(), 4);
        assert!(snapshot.ids.contains_key("ListOwnerships"));

        // ... while a repeat of the required refresh is.
        let requests = transport.requests.lock().expect("requests").len();
        store
            .refresh_required_with_cookies(&transport, &operations, "HomeTimeline", COOKIE)
            .expect("repeat required refresh");
        assert_eq!(transport.requests.lock().expect("requests").len(), requests);
    }

    #[test]
    fn required_refresh_scans_lazy_chunks_only_until_it_finds_the_operation() {
        let temp = tempfile::tempdir().expect("temp directory");
        let transport = ChunkTransport::new(40, &[("Chunk002", LISTS_BODY)]);
        let snapshot = temp_store(&temp)
            .refresh_required_with_cookies(
                &transport,
                &names(&["HomeTimeline", "ListOwnerships", "Likes"]),
                "ListOwnerships",
                COOKIE,
            )
            .expect("required refresh");

        assert!(snapshot.ids.contains_key("ListOwnerships"));
        assert_eq!(transport.chunk_requests(), 3);
    }

    #[test]
    fn repeat_refresh_for_other_operations_is_not_answered_from_the_cooldown() {
        let temp = tempfile::tempdir().expect("temp directory");
        let store = temp_store(&temp);
        let transport = ChunkTransport::new(5, &[("Chunk001", LISTS_BODY)]);

        store
            .refresh_with_cookies(&transport, &names(&["HomeTimeline"]), COOKIE)
            .expect("first refresh");
        let snapshot = store
            .refresh_with_cookies(&transport, &names(&["ListOwnerships"]), COOKIE)
            .expect("second refresh");

        assert!(snapshot.ids.contains_key("ListOwnerships"));
    }

    #[test]
    fn signed_in_refresh_is_not_answered_by_a_logged_out_one() {
        let temp = tempfile::tempdir().expect("temp directory");
        let store = temp_store(&temp);
        let transport = SignedInTransport::default();
        let operations = names(&["HomeTimeline"]);

        let error = store
            .refresh_with_cookies(&transport, &operations, None)
            .expect_err("the logged-out client carries no bundles");
        assert!(error.to_string().contains("signed-in session"));
        // Logged out, the first page without bundles ends the search.
        assert_eq!(transport.requests.load(Ordering::SeqCst), 1);

        store
            .refresh_with_cookies(&transport, &operations, COOKIE)
            .expect("signed-in refresh");
    }

    #[test]
    fn failed_refresh_is_not_repeated_right_away() {
        let temp = tempfile::tempdir().expect("temp directory");
        let store = temp_store(&temp);
        let transport = ChunkTransport::new(0, &[]);
        let operations = names(&["Likes"]);

        let first = store
            .refresh_with_cookies(&transport, &operations, COOKIE)
            .expect_err("nothing to discover");
        assert!(
            first
                .to_string()
                .contains("No GraphQL query IDs discovered")
        );
        let requests = transport.requests.lock().expect("requests").len();

        let second = store
            .refresh_with_cookies(&transport, &operations, COOKIE)
            .expect_err("failure is remembered");
        assert!(
            second
                .to_string()
                .contains("No GraphQL query IDs discovered")
        );
        assert!(second.to_string().contains("not retrying yet"));
        assert_eq!(transport.requests.lock().expect("requests").len(), requests);

        // Other operations may still be found, so they are not turned away.
        store
            .refresh_with_cookies(&transport, &names(&["HomeTimeline"]), COOKIE)
            .expect("a different request runs");
    }

    #[test]
    fn lazy_scan_gives_up_when_the_network_stops_answering() {
        let temp = tempfile::tempdir().expect("temp directory");
        let mut transport = ChunkTransport::new(100, &[]);
        transport.chunk_errors = true;

        let snapshot = temp_store(&temp)
            .refresh_with_cookies(
                &transport,
                &names(&["HomeTimeline", "ListOwnerships"]),
                COOKIE,
            )
            .expect("the entry bundle still yields HomeTimeline");

        assert!(snapshot.ids.contains_key("HomeTimeline"));
        assert_eq!(transport.chunk_requests(), MAX_LAZY_ERRORS);
    }

    #[test]
    fn discovery_makes_no_request_once_its_budget_is_spent() {
        let transport = ChunkTransport::new(5, &[]);
        let bundles = DiscoveredBundles {
            pages: Vec::new(),
            entry: vec!["https://abs.twimg.com/responsive-web/client-web/main.abcdef01.js".into()],
            lazy: vec![
                "https://abs.twimg.com/responsive-web/client-web/shared~bundle.Chunk000.0000000a.js"
                    .into(),
            ],
        };

        let extraction = fetch_and_extract(
            &transport,
            &bundles,
            &names(&["HomeTimeline"]),
            None,
            Deadline::after(Duration::ZERO),
        )
        .expect("extraction");

        assert!(extraction.ids.is_empty());
        assert!(transport.requests.lock().expect("requests").is_empty());
    }

    #[test]
    fn chunk_ids_in_exponent_notation_keep_their_own_hash() {
        // 9000 and 79000 are minified to `9e3` and `79e3`: both must not collapse into `3`.
        let html = r#"<script src="https://abs.twimg.com/responsive-web/client-web/main.abcdef01.js"></script>
            t.u=e=>(({9e3:"loader.SignupModule",79e3:"bundle.Grok",5:"bundle.Home"})[e]||e)
            +"."+({9e3:"aaaaaaaaaaaaaaaa",79e3:"bbbbbbbbbbbbbbbb",5:"ccccccc"})[e]+"a.js""#;

        assert_eq!(
            chunk_hashes(html),
            BTreeMap::from([
                ("9e3", "aaaaaaaaaaaaaaaa"),
                ("79e3", "bbbbbbbbbbbbbbbb"),
                ("5", "ccccccc")
            ])
        );
        let urls = discover_lazy_chunks(html);
        for expected in [
            "/loader.SignupModule.aaaaaaaaaaaaaaaaa.js",
            "/bundle.Grok.bbbbbbbbbbbbbbbba.js",
            "/bundle.Home.ccccccca.js",
        ] {
            assert!(urls.iter().any(|url| url.ends_with(expected)), "{urls:?}");
        }
    }

    struct QueryIdTransport;
    struct EmptyQueryIdTransport;
    struct UnrelatedQueryIdTransport;

    impl HttpTransport for QueryIdTransport {
        fn send(&self, request: &HttpRequest) -> anyhow::Result<HttpResponse> {
            let body = if request
                .url
                .ends_with("shared~bundle.BookmarkFolders~bundle.Bookmarks.e9764eba.js")
            {
                br#"e.exports={queryId:"new-bookmarks-id",operationName:"Bookmarks"}"#.to_vec()
            } else if request.url.ends_with(".js") {
                Vec::new()
            } else {
                br#"
                    <script src="https://abs.twimg.com/responsive-web/client-web/main.abcdef01.js"></script>
                    <script>
                    e.u=e=>({34778:"shared~bundle.BookmarkFolders~bundle.Bookmarks"})[e]
                        +"."+({34778:"e9764eb"})[e]+"a.js"
                    </script>
                "#
                .to_vec()
            };
            Ok(HttpResponse {
                status: 200,
                body,
                ..HttpResponse::default()
            })
        }
    }

    impl HttpTransport for EmptyQueryIdTransport {
        fn send(&self, request: &HttpRequest) -> anyhow::Result<HttpResponse> {
            let body = if request.url.ends_with(".js") {
                Vec::new()
            } else {
                br#"<script src="https://abs.twimg.com/responsive-web/client-web/main.abcdef01.js"></script>"#
                    .to_vec()
            };
            Ok(HttpResponse {
                status: 200,
                body,
                ..HttpResponse::default()
            })
        }
    }

    impl HttpTransport for UnrelatedQueryIdTransport {
        fn send(&self, request: &HttpRequest) -> anyhow::Result<HttpResponse> {
            let body = if request.url.ends_with(".js") {
                br#"e.exports={queryId:"create-tweet-id",operationName:"CreateTweet"}"#.to_vec()
            } else {
                br#"<script src="https://abs.twimg.com/responsive-web/client-web/main.abcdef01.js"></script>"#
                    .to_vec()
            };
            Ok(HttpResponse {
                status: 200,
                body,
                ..HttpResponse::default()
            })
        }
    }

    #[test]
    fn discovers_lazy_bookmark_bundle_from_chunk_manifest() {
        let html = r#"
            <script src="https://abs.twimg.com/responsive-web/client-web/main.abcdef01.js"></script>
            <script>
            e.u=e=>({34778:"shared~bundle.BookmarkFolders~bundle.Bookmarks"})[e]
                +"."+({34778:"e9764eb"})[e]+"a.js"
            </script>
        "#;

        assert_eq!(
            discover_lazy_bookmark_bundles(html),
            vec![
                "https://abs.twimg.com/responsive-web/client-web/shared~bundle.BookmarkFolders~bundle.Bookmarks.e9764eba.js"
                    .to_owned()
            ]
        );
    }

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

    #[test]
    fn refresh_replaces_the_in_memory_snapshot() {
        let temp = tempfile::tempdir().expect("temp directory");
        let cache_path = temp.path().join("query-ids.json");
        let snapshot = Snapshot {
            fetched_at: "2026-07-25T00:00:00.000Z".to_owned(),
            ttl_ms: 60_000,
            ids: BTreeMap::from([
                ("Bookmarks".to_owned(), "old-bookmarks-id".to_owned()),
                ("HomeTimeline".to_owned(), "keep-home-id".to_owned()),
            ]),
            discovery: Discovery {
                pages: Vec::new(),
                bundles: Vec::new(),
            },
        };
        fs::write(
            &cache_path,
            serde_json::to_string(&snapshot).expect("snapshot JSON"),
        )
        .expect("cache write");
        let store = RuntimeQueryIdStore::new(Some(cache_path), Some(Duration::from_secs(60)));

        assert_eq!(
            store.get_query_id("Bookmarks").as_deref(),
            Some("old-bookmarks-id")
        );
        store
            .refresh(&QueryIdTransport, &["Bookmarks".to_owned()])
            .expect("refresh");
        assert_eq!(
            store.get_query_id("Bookmarks").as_deref(),
            Some("new-bookmarks-id")
        );
        assert_eq!(
            store.get_query_id("HomeTimeline").as_deref(),
            Some("keep-home-id")
        );
    }

    #[test]
    fn refresh_rejects_an_empty_discovery() {
        let temp = tempfile::tempdir().expect("temp directory");
        let store = RuntimeQueryIdStore::new(
            Some(temp.path().join("query-ids.json")),
            Some(Duration::from_secs(60)),
        );

        let error = store
            .refresh(&EmptyQueryIdTransport, &["Bookmarks".to_owned()])
            .expect_err("empty discovery should fail");

        assert!(
            error
                .to_string()
                .contains("No GraphQL query IDs discovered")
        );
    }

    #[test]
    fn required_refresh_rejects_an_unrelated_discovery() {
        let temp = tempfile::tempdir().expect("temp directory");
        let store = RuntimeQueryIdStore::new(
            Some(temp.path().join("query-ids.json")),
            Some(Duration::from_secs(60)),
        );

        let error = store
            .refresh_required(
                &UnrelatedQueryIdTransport,
                &["Bookmarks".to_owned(), "CreateTweet".to_owned()],
                "Bookmarks",
            )
            .expect_err("required operation should be newly discovered");

        assert!(error.to_string().contains("current Bookmarks query"));
    }
}
