use std::sync::{LazyLock, Mutex};
use std::thread;
use std::time::Duration;

use anyhow::Context;
use rand::Rng;
use regex::Regex;
use serde_json::{json, Value};
use url::form_urlencoded::Serializer;
use uuid::Uuid;

use crate::credentials::default_user_agent;
use crate::features::{
    build_article_field_toggles, build_bookmarks_features, build_explore_features,
    build_following_features, build_home_timeline_features, build_likes_features,
    build_lists_features, build_search_features, build_tweet_create_features,
    build_tweet_detail_features, build_user_tweets_features,
};
use crate::parser::{
    extract_cursor_from_instructions, find_tweet_in_instructions, map_tweet_result,
    normalize_quote_depth, parse_lists_from_instructions, parse_tweets_from_instructions,
    parse_users_from_instructions,
};
use crate::query_ids::{target_query_id_operations, RuntimeQueryIdStore};
use crate::transaction_id::RuntimeTransactionIdStore;
use crate::transport::{CurlTransport, HttpRequest, HttpTransport};
use crate::types::{
    AboutProfile, BookmarkMutationResult, CurrentUser, FollowMutationResult, MediaUploadResult,
    NewsItem, QueryIdSnapshot, TweetData, TweetMutationResult, TweetsPage,
    TwitterClientOptions, TwitterCookies, TwitterList, TwitterUser, UsersPage,
};

const TWITTER_API_BASE: &str = "https://x.com/i/api/graphql";
const TWITTER_GRAPHQL_POST_URL: &str = "https://x.com/i/api/graphql";
const TWITTER_UPLOAD_URL: &str = "https://upload.twitter.com/i/media/upload.json";
const TWITTER_MEDIA_METADATA_URL: &str = "https://x.com/i/api/1.1/media/metadata/create.json";
const TWITTER_STATUS_UPDATE_URL: &str = "https://x.com/i/api/1.1/statuses/update.json";
const BEARER_TOKEN: &str = "AAAAAAAAAAAAAAAAAAAAANRILgAAAAAAnNwIzUejRCOuH5E6I8xnZz4puTs%3D1Zv7ttfk8LF81IUq16cHjhLTvJu4FA33AGWWjCpTnA";
const SETTINGS_SCREEN_NAME_REGEX: &str = r#""screen_name":"([^"]+)""#;
const SETTINGS_USER_ID_REGEX: &str = r#""user_id"\s*:\s*"(\d+)""#;
const SETTINGS_NAME_REGEX: &str = r#""name":"([^"\\]*(?:\\.[^"\\]*)*)""#;

static SETTINGS_SCREEN_NAME_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(SETTINGS_SCREEN_NAME_REGEX).expect("valid regex"));
static SETTINGS_USER_ID_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(SETTINGS_USER_ID_REGEX).expect("valid regex"));
static SETTINGS_NAME_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(SETTINGS_NAME_REGEX).expect("valid regex"));
static CHROME_VERSION_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"Chrome/(\d+)").expect("valid regex"));
static POST_COUNT_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)([\d.]+)([KMB]?)\s*posts?").expect("valid regex"));
const EXPLORE_TAB_FOR_YOU: &str = "forYou";
const EXPLORE_TAB_TRENDING: &str = "trending";
const EXPLORE_TAB_NEWS: &str = "news";
const EXPLORE_TAB_SPORTS: &str = "sports";
const EXPLORE_TAB_ENTERTAINMENT: &str = "entertainment";
const EXPLORE_TIMELINE_FOR_YOU: &str = "VGltZWxpbmU6DAC2CwABAAAAB2Zvcl95b3UAAA==";
const EXPLORE_TIMELINE_TRENDING: &str = "VGltZWxpbmU6DAC2CwABAAAACHRyZW5kaW5nAAA=";
const EXPLORE_TIMELINE_NEWS: &str = "VGltZWxpbmU6DAC2CwABAAAABG5ld3MAAA==";
const EXPLORE_TIMELINE_SPORTS: &str = "VGltZWxpbmU6DAC2CwABAAAABnNwb3J0cwAA";
const EXPLORE_TIMELINE_ENTERTAINMENT: &str = "VGltZWxpbmU6DAC2CwABAAAADWVudGVydGFpbm1lbnQAAA==";

pub struct TwitterClient {
    cookies: TwitterCookies,
    user_agent: String,
    timeout: Option<Duration>,
    quote_depth: usize,
    transport: CurlTransport,
    query_ids: RuntimeQueryIdStore,
    transaction_ids: RuntimeTransactionIdStore,
    client_uuid: String,
    client_device_id: String,
    client_user_id: Mutex<Option<String>>,
}

impl TwitterClient {
    pub fn new(options: TwitterClientOptions) -> anyhow::Result<Self> {
        if options.cookies.auth_token.is_none() || options.cookies.ct0.is_none() {
            anyhow::bail!("Both auth_token and ct0 cookies are required");
        }
        Ok(Self {
            cookies: options.cookies,
            user_agent: options
                .user_agent
                .unwrap_or_else(|| default_user_agent().to_owned()),
            timeout: options.timeout,
            quote_depth: normalize_quote_depth(options.quote_depth),
            transport: CurlTransport::new(std::env::var("TWITTER_PROXY").ok()),
            query_ids: RuntimeQueryIdStore::default(),
            transaction_ids: RuntimeTransactionIdStore::default(),
            client_uuid: Uuid::new_v4().to_string(),
            client_device_id: Uuid::new_v4().to_string(),
            client_user_id: Mutex::new(None),
        })
    }

    pub fn query_ids_snapshot(&self) -> QueryIdSnapshot {
        self.query_ids.snapshot()
    }

    pub fn refresh_query_ids(&self) -> anyhow::Result<QueryIdSnapshot> {
        self.query_ids
            .refresh(&self.transport, &target_query_id_operations())
    }

    pub fn tweet(&self, text: &str, media_ids: Option<&[String]>) -> TweetMutationResult {
        let variables = json!({
            "tweet_text": text,
            "dark_request": false,
            "media": {
                "media_entities": media_entities(media_ids),
                "possibly_sensitive": false
            },
            "semantic_annotation_ids": []
        });
        self.create_tweet(variables, build_tweet_create_features())
    }

    pub fn reply(
        &self,
        text: &str,
        reply_to_tweet_id: &str,
        media_ids: Option<&[String]>,
    ) -> TweetMutationResult {
        let variables = json!({
            "tweet_text": text,
            "reply": {
                "in_reply_to_tweet_id": reply_to_tweet_id,
                "exclude_reply_user_ids": []
            },
            "dark_request": false,
            "media": {
                "media_entities": media_entities(media_ids),
                "possibly_sensitive": false
            },
            "semantic_annotation_ids": []
        });
        self.create_tweet(variables, build_tweet_create_features())
    }

    pub fn upload_media(
        &self,
        data: &[u8],
        mime_type: &str,
        alt: Option<&str>,
    ) -> MediaUploadResult {
        let Some(category) = media_category_for_mime(mime_type) else {
            return MediaUploadResult {
                success: false,
                media_id: None,
                error: Some(format!("Unsupported media type: {mime_type}")),
            };
        };

        match self.upload_media_impl(data, mime_type, category, alt) {
            Ok(media_id) => MediaUploadResult {
                success: true,
                media_id: Some(media_id),
                error: None,
            },
            Err(error) => MediaUploadResult {
                success: false,
                media_id: None,
                error: Some(error.to_string()),
            },
        }
    }

    pub fn unbookmark(&self, tweet_id: &str) -> BookmarkMutationResult {
        self.perform_engagement_mutation("DeleteBookmark", tweet_id, false)
    }

    pub fn follow(&self, user_id: &str) -> FollowMutationResult {
        let result: anyhow::Result<FollowMutationResult> = (|| {
            self.ensure_client_user_id()?;
            let rest = self.follow_via_rest(user_id, "create")?;
            if rest.success {
                return Ok(rest);
            }
            self.follow_via_graphql(user_id, true)
        })();
        result.unwrap_or_else(|error| FollowMutationResult {
            success: false,
            user_id: None,
            username: None,
            error: Some(error.to_string()),
        })
    }

    pub fn unfollow(&self, user_id: &str) -> FollowMutationResult {
        let result: anyhow::Result<FollowMutationResult> = (|| {
            self.ensure_client_user_id()?;
            let rest = self.follow_via_rest(user_id, "destroy")?;
            if rest.success {
                return Ok(rest);
            }
            self.follow_via_graphql(user_id, false)
        })();
        result.unwrap_or_else(|error| FollowMutationResult {
            success: false,
            user_id: None,
            username: None,
            error: Some(error.to_string()),
        })
    }

    pub fn get_current_user(&self) -> anyhow::Result<CurrentUser> {
        let urls = [
            "https://x.com/i/api/account/settings.json",
            "https://api.twitter.com/1.1/account/settings.json",
            "https://x.com/i/api/account/verify_credentials.json?skip_status=true&include_entities=false",
            "https://api.twitter.com/1.1/account/verify_credentials.json?skip_status=true&include_entities=false",
        ];
        let mut last_error = None;
        for url in urls {
            match self.send_request("GET", url, self.headers_json(), None) {
                Ok(response) if response.is_success() => {
                    let data = response.json()?;
                    let username = first_string(&[
                        data.get("screen_name"),
                        data.get("user").and_then(|value| value.get("screen_name")),
                    ]);
                    let name = first_string(&[
                        data.get("name"),
                        data.get("user").and_then(|value| value.get("name")),
                    ]);
                    let user_id = first_string(&[
                        data.get("user_id"),
                        data.get("user_id_str"),
                        data.get("user").and_then(|value| value.get("id_str")),
                        data.get("user").and_then(|value| value.get("id")),
                    ]);
                    if let (Some(username), Some(user_id)) = (username, user_id) {
                        let user = CurrentUser {
                            id: user_id.clone(),
                            username: username.clone(),
                            name: name.unwrap_or(username),
                        };
                        self.client_user_id.lock().ok().map(|mut slot| *slot = Some(user_id));
                        return Ok(user);
                    }
                    last_error = Some(anyhow::anyhow!("Could not determine current user from response"));
                }
                Ok(response) => {
                    last_error = Some(anyhow::anyhow!(
                        "HTTP {}: {}",
                        response.status,
                        truncate(&response.text(), 200)
                    ));
                }
                Err(error) => last_error = Some(error),
            }
        }

        let screen_name_re = &*SETTINGS_SCREEN_NAME_RE;
        let user_id_re = &*SETTINGS_USER_ID_RE;
        let name_re = &*SETTINGS_NAME_RE;
        for url in ["https://x.com/settings/account", "https://twitter.com/settings/account"] {
            match self.send_request(
                "GET",
                url,
                vec![
                    ("cookie".into(), self.cookie_header()),
                    ("user-agent".into(), self.user_agent.clone()),
                ],
                None,
            ) {
                Ok(response) if response.is_success() => {
                    let html = response.text();
                    let username = screen_name_re
                        .captures(&html)
                        .and_then(|capture| capture.get(1))
                        .map(|value| value.as_str().to_owned());
                    let user_id = user_id_re
                        .captures(&html)
                        .and_then(|capture| capture.get(1))
                        .map(|value| value.as_str().to_owned());
                    let name = name_re
                        .captures(&html)
                        .and_then(|capture| capture.get(1))
                        .map(|value| value.as_str().replace("\\\"", "\""));
                    if let (Some(username), Some(user_id)) = (username, user_id) {
                        self.client_user_id.lock().ok().map(|mut slot| *slot = Some(user_id.clone()));
                        return Ok(CurrentUser {
                            id: user_id,
                            username: username.clone(),
                            name: name.unwrap_or(username),
                        });
                    }
                }
                Ok(response) => {
                    last_error = Some(anyhow::anyhow!("HTTP {} (settings page)", response.status));
                }
                Err(error) => last_error = Some(error),
            }
        }
        Err(last_error.unwrap_or_else(|| anyhow::anyhow!("Unknown error fetching current user")))
    }

    pub fn get_likes(
        &self,
        count: usize,
        include_raw: bool,
        cursor: Option<String>,
        max_pages: Option<usize>,
    ) -> anyhow::Result<TweetsPage> {
        let user_id = self.client_user_id_value()?;
        let features = build_likes_features();
        let page_size = 20usize;
        let unlimited = max_pages.is_some();
        let mut seen = std::collections::BTreeSet::new();
        let mut tweets = Vec::new();
        let mut cursor = cursor;
        let mut next_cursor = None;
        let mut pages_fetched = 0usize;

        while unlimited || tweets.len() < count {
            let page_count = if unlimited {
                page_size
            } else {
                page_size.min(count.saturating_sub(tweets.len()))
            };
            let variables = json!({
                "userId": user_id,
                "count": page_count,
                "includePromotedContent": false,
                "withClientEventToken": false,
                "withBirdwatchNotes": false,
                "withVoice": true,
                "cursor": cursor
            });
            let params = encode_params(&[
                ("variables", serde_json::to_string(&variables)?),
                ("features", serde_json::to_string(&features)?),
            ]);
            let page = self.fetch_tweet_timeline_page(
                &self.likes_query_ids(),
                "Likes",
                &params,
                &["data", "user", "result", "timeline", "timeline", "instructions"],
                include_raw,
                true,
            )?;
            pages_fetched += 1;
            let mut added = 0usize;
            for tweet in page.tweets {
                if seen.insert(tweet.id.clone()) {
                    tweets.push(tweet);
                    added += 1;
                    if !unlimited && tweets.len() >= count {
                        break;
                    }
                }
            }
            if page.cursor.is_none()
                || page.cursor == cursor
                || added == 0
                || max_pages.map(|max| pages_fetched >= max).unwrap_or(false)
            {
                next_cursor = page.cursor;
                break;
            }
            cursor = page.cursor.clone();
            next_cursor = cursor.clone();
        }

        Ok(TweetsPage { tweets, next_cursor })
    }

    pub fn get_bookmarks(
        &self,
        count: usize,
        include_raw: bool,
        cursor: Option<String>,
        max_pages: Option<usize>,
    ) -> anyhow::Result<TweetsPage> {
        let features = build_bookmarks_features();
        let page_size = 20usize;
        let unlimited = max_pages.is_some() || cursor.is_some();
        let mut seen = std::collections::BTreeSet::new();
        let mut tweets = Vec::new();
        let mut cursor = cursor;
        let mut next_cursor = None;
        let mut pages_fetched = 0usize;

        while unlimited || tweets.len() < count {
            let page_count = if unlimited {
                page_size
            } else {
                page_size.min(count.saturating_sub(tweets.len()))
            };
            let variables = json!({
                "count": page_count,
                "includePromotedContent": false,
                "withDownvotePerspective": false,
                "withReactionsMetadata": false,
                "withReactionsPerspective": false,
                "cursor": cursor
            });
            let params = encode_params(&[
                ("variables", serde_json::to_string(&variables)?),
                ("features", serde_json::to_string(&features)?),
            ]);
            let page = self.fetch_tweet_timeline_page_with_retry(
                &self.bookmarks_query_ids(),
                "Bookmarks",
                &params,
                &["data", "bookmark_timeline_v2", "timeline", "instructions"],
                include_raw,
                true,
            )?;
            let page_was_empty = page.tweets.is_empty();
            pages_fetched += 1;
            let mut added = 0usize;
            for tweet in page.tweets {
                if seen.insert(tweet.id.clone()) {
                    tweets.push(tweet);
                    added += 1;
                    if !unlimited && tweets.len() >= count {
                        break;
                    }
                }
            }
            if page.cursor.is_none()
                || page.cursor == cursor
                || page_was_empty
                || added == 0
                || max_pages.map(|max| pages_fetched >= max).unwrap_or(false)
            {
                next_cursor = if max_pages.map(|max| pages_fetched >= max).unwrap_or(false) {
                    page.cursor
                } else {
                    None
                };
                break;
            }
            cursor = page.cursor.clone();
            next_cursor = cursor.clone();
        }

        Ok(TweetsPage { tweets, next_cursor })
    }

    pub fn get_bookmark_folder_timeline(
        &self,
        folder_id: &str,
        count: usize,
        include_raw: bool,
        cursor: Option<String>,
        max_pages: Option<usize>,
    ) -> anyhow::Result<TweetsPage> {
        let features = build_bookmarks_features();
        let page_size = 20usize;
        let unlimited = max_pages.is_some() || cursor.is_some();
        let mut seen = std::collections::BTreeSet::new();
        let mut tweets = Vec::new();
        let mut cursor = cursor;
        let mut next_cursor = None;
        let mut pages_fetched = 0usize;

        while unlimited || tweets.len() < count {
            let page_count = if unlimited {
                page_size
            } else {
                page_size.min(count.saturating_sub(tweets.len()))
            };
            let page = self.fetch_bookmark_folder_page(
                folder_id,
                page_count,
                cursor.clone(),
                &features,
                include_raw,
            )?;
            let page_was_empty = page.tweets.is_empty();
            pages_fetched += 1;
            let mut added = 0usize;
            for tweet in page.tweets {
                if seen.insert(tweet.id.clone()) {
                    tweets.push(tweet);
                    added += 1;
                    if !unlimited && tweets.len() >= count {
                        break;
                    }
                }
            }
            if page.cursor.is_none()
                || page.cursor == cursor
                || page_was_empty
                || added == 0
                || max_pages.map(|max| pages_fetched >= max).unwrap_or(false)
            {
                next_cursor = if max_pages.map(|max| pages_fetched >= max).unwrap_or(false) {
                    page.cursor
                } else {
                    None
                };
                break;
            }
            cursor = page.cursor.clone();
            next_cursor = cursor.clone();
        }

        Ok(TweetsPage { tweets, next_cursor })
    }

    pub fn get_following(
        &self,
        user_id: &str,
        count: usize,
        cursor: Option<String>,
        max_pages: Option<usize>,
    ) -> anyhow::Result<UsersPage> {
        self.get_users_page(user_id, count, cursor, max_pages, true)
    }

    pub fn get_followers(
        &self,
        user_id: &str,
        count: usize,
        cursor: Option<String>,
        max_pages: Option<usize>,
    ) -> anyhow::Result<UsersPage> {
        self.get_users_page(user_id, count, cursor, max_pages, false)
    }

    pub fn get_user_about_account(&self, username: &str) -> anyhow::Result<AboutProfile> {
        let clean = normalize_handle(username).context("Invalid username")?;
        let variables = json!({
            "screenName": clean
        });
        let params = encode_params(&[("variables", serde_json::to_string(&variables)?)]);

        self.with_refreshed_query_ids_on_error(|| {
            let mut last_error = None;
            for query_id in self.about_account_query_ids() {
                let url = format!("{TWITTER_API_BASE}/{query_id}/AboutAccountQuery?{params}");
                let response = self.send_request("GET", &url, self.headers_json(), None);
                let Ok(response) = response else {
                    last_error = Some(response.err().unwrap().to_string());
                    continue;
                };
                if response.status == 404 {
                    last_error = Some("HTTP 404".to_owned());
                    continue;
                }
                if !response.is_success() {
                    last_error = Some(format!(
                        "HTTP {}: {}",
                        response.status,
                        truncate(&response.text(), 200)
                    ));
                    continue;
                }
                let data = response.json().map_err(|error| RefreshableError {
                    message: error.to_string(),
                    needs_refresh: false,
                })?;
                if let Some(errors) = data.get("errors").and_then(Value::as_array) {
                    last_error = Some(format_errors(errors));
                    continue;
                }
                let about_profile = data
                    .get("data")
                    .and_then(|value| value.get("user_result_by_screen_name"))
                    .and_then(|value| value.get("result"))
                    .and_then(|value| value.get("about_profile"))
                    .ok_or_else(|| RefreshableError {
                        message: "Missing about_profile in response".to_owned(),
                        needs_refresh: false,
                    })?;

                return Ok(AboutProfile {
                    account_based_in: about_profile
                        .get("account_based_in")
                        .and_then(Value::as_str)
                        .map(ToOwned::to_owned),
                    source: about_profile
                        .get("source")
                        .and_then(Value::as_str)
                        .map(ToOwned::to_owned),
                    created_country_accurate: about_profile
                        .get("created_country_accurate")
                        .and_then(Value::as_bool),
                    location_accurate: about_profile
                        .get("location_accurate")
                        .and_then(Value::as_bool),
                    learn_more_url: about_profile
                        .get("learn_more_url")
                        .and_then(Value::as_str)
                        .map(ToOwned::to_owned),
                });
            }

            Err(RefreshableError {
                message: last_error.unwrap_or_else(|| "Unknown error fetching account details".to_owned()),
                needs_refresh: true,
            })
        })
        .map_err(anyhow::Error::msg)
    }

    pub fn get_owned_lists(&self, count: usize) -> anyhow::Result<Vec<TwitterList>> {
        let user_id = self.client_user_id_value()?;
        self.get_lists_for_user("ListOwnerships", &self.list_ownerships_query_ids(), &user_id, count)
    }

    pub fn get_list_timeline(
        &self,
        list_id: &str,
        count: usize,
        include_raw: bool,
        cursor: Option<String>,
        max_pages: Option<usize>,
    ) -> anyhow::Result<TweetsPage> {
        let features = build_lists_features();
        let page_size = 20usize;
        let unlimited = max_pages.is_some();
        let mut seen = std::collections::BTreeSet::new();
        let mut tweets = Vec::new();
        let mut cursor = cursor;
        let mut next_cursor = None;
        let mut pages_fetched = 0usize;

        while unlimited || tweets.len() < count {
            let page_count = if unlimited {
                page_size
            } else {
                page_size.min(count.saturating_sub(tweets.len()))
            };
            let variables = json!({
                "listId": list_id,
                "count": page_count,
                "cursor": cursor
            });
            let params = encode_params(&[
                ("variables", serde_json::to_string(&variables)?),
                ("features", serde_json::to_string(&features)?),
            ]);
            let page = self.fetch_tweet_timeline_page(
                &self.list_timeline_query_ids(),
                "ListLatestTweetsTimeline",
                &params,
                &["data", "list", "tweets_timeline", "timeline", "instructions"],
                include_raw,
                false,
            )?;
            pages_fetched += 1;
            let mut added = 0usize;
            for tweet in page.tweets {
                if seen.insert(tweet.id.clone()) {
                    tweets.push(tweet);
                    added += 1;
                    if !unlimited && tweets.len() >= count {
                        break;
                    }
                }
            }
            if page.cursor.is_none()
                || page.cursor == cursor
                || added == 0
                || max_pages.map(|max| pages_fetched >= max).unwrap_or(false)
            {
                next_cursor = page.cursor;
                break;
            }
            cursor = page.cursor.clone();
            next_cursor = cursor.clone();
        }

        Ok(TweetsPage { tweets, next_cursor })
    }

    pub fn get_news(
        &self,
        count: usize,
        include_raw: bool,
        with_tweets: bool,
        tweets_per_item: usize,
        ai_only: bool,
        tabs: Option<Vec<String>>,
    ) -> anyhow::Result<Vec<NewsItem>> {
        let requested_tabs = tabs.unwrap_or_else(|| {
            vec![
                EXPLORE_TAB_FOR_YOU.to_owned(),
                EXPLORE_TAB_NEWS.to_owned(),
                EXPLORE_TAB_SPORTS.to_owned(),
                EXPLORE_TAB_ENTERTAINMENT.to_owned(),
            ]
        });
        let mut items = Vec::new();
        let mut seen = std::collections::BTreeSet::new();

        for tab in requested_tabs {
            let Some(timeline_id) = explore_timeline_id(&tab) else {
                continue;
            };
            let tab_items = self.fetch_news_tab(&tab, timeline_id, count, ai_only, include_raw)?;
            for item in tab_items {
                if seen.insert(item.headline.clone()) {
                    items.push(item);
                    if items.len() >= count {
                        break;
                    }
                }
            }
            if items.len() >= count {
                break;
            }
        }

        if items.is_empty() {
            anyhow::bail!("No news items found");
        }

        items.truncate(count);
        if with_tweets {
            for item in &mut items {
                let tweets = self
                    .search(&item.headline, tweets_per_item, include_raw, None, None)?
                    .tweets;
                if !tweets.is_empty() {
                    item.tweets = Some(tweets);
                }
            }
        }

        Ok(items)
    }

    fn create_tweet(&self, variables: Value, features: Value) -> TweetMutationResult {
        let result: anyhow::Result<TweetMutationResult> = (|| {
            self.ensure_client_user_id()?;
            let mut query_id = self.query_id("CreateTweet");
            let body_for = |query_id: &str| {
                serde_json::to_vec(&json!({
                    "variables": variables.clone(),
                    "features": features.clone(),
                    "queryId": query_id
                }))
            };
            let send_create = |url: &str, body: Vec<u8>| {
                self.send_request(
                    "POST",
                    url,
                    self.headers_json_with_referer("https://x.com/compose/post"),
                    Some(body),
                )
            };

            let mut operation_url = format!("{TWITTER_API_BASE}/{query_id}/CreateTweet");
            let mut response = send_create(&operation_url, body_for(&query_id)?)?;

            if response.status == 404 {
                let _ = self.refresh_query_ids();
                query_id = self.query_id("CreateTweet");
                operation_url = format!("{TWITTER_API_BASE}/{query_id}/CreateTweet");
                response = send_create(&operation_url, body_for(&query_id)?)?;

                if response.status == 404 {
                    response = send_create(TWITTER_GRAPHQL_POST_URL, body_for(&query_id)?)?;
                }
            }

            self.parse_create_tweet_response(response, &variables)
        })();

        match result {
            Ok(result) => result,
            Err(error) => TweetMutationResult {
                success: false,
                tweet_id: None,
                error: Some(error.to_string()),
            },
        }
    }

    fn perform_engagement_mutation(
        &self,
        operation_name: &str,
        tweet_id: &str,
        ensure_user_id: bool,
    ) -> BookmarkMutationResult {
        let result: anyhow::Result<BookmarkMutationResult> = (|| {
            if ensure_user_id {
                self.ensure_client_user_id()?;
            }
            let variables = if operation_name == "DeleteRetweet" {
                json!({
                    "tweet_id": tweet_id,
                    "source_tweet_id": tweet_id
                })
            } else {
                json!({
                    "tweet_id": tweet_id
                })
            };
            let body_for = |query_id: &str| {
                serde_json::to_vec(&json!({
                    "variables": variables.clone(),
                    "queryId": query_id
                }))
            };
            let headers = self.headers_json_with_referer(&format!("https://x.com/i/status/{tweet_id}"));

            let mut query_id = self.query_id(operation_name);
            let mut operation_url = format!("{TWITTER_API_BASE}/{query_id}/{operation_name}");
            let mut response =
                self.send_request("POST", &operation_url, headers.clone(), Some(body_for(&query_id)?))?;
            if response.status == 404 {
                let _ = self.refresh_query_ids();
                query_id = self.query_id(operation_name);
                operation_url = format!("{TWITTER_API_BASE}/{query_id}/{operation_name}");
                response = self.send_request(
                    "POST",
                    &operation_url,
                    headers.clone(),
                    Some(body_for(&query_id)?),
                )?;
                if response.status == 404 {
                    response = self.send_request(
                        "POST",
                        TWITTER_GRAPHQL_POST_URL,
                        headers,
                        Some(body_for(&query_id)?),
                    )?;
                }
            }

            parse_bookmark_mutation_response(response)
        })();

        result.unwrap_or_else(|error| BookmarkMutationResult {
            success: false,
            error: Some(error.to_string()),
        })
    }

    fn parse_create_tweet_response(
        &self,
        response: crate::transport::HttpResponse,
        variables: &Value,
    ) -> anyhow::Result<TweetMutationResult> {
        if !response.is_success() {
            return Ok(TweetMutationResult {
                success: false,
                tweet_id: None,
                error: Some(format!(
                    "HTTP {}: {}",
                    response.status,
                    truncate(&response.text(), 200)
                )),
            });
        }

        let data = response.json()?;
        if let Some(errors) = data.get("errors").and_then(Value::as_array) {
            if let Some(fallback) = self.try_status_update_fallback(errors, variables)? {
                return Ok(fallback);
            }
            return Ok(TweetMutationResult {
                success: false,
                tweet_id: None,
                error: Some(format_errors(errors)),
            });
        }

        let tweet_id = data
            .get("data")
            .and_then(|value| value.get("create_tweet"))
            .and_then(|value| value.get("tweet_results"))
            .and_then(|value| value.get("result"))
            .and_then(|value| value.get("rest_id"))
            .and_then(Value::as_str)
            .map(ToOwned::to_owned);

        Ok(match tweet_id {
            Some(tweet_id) => TweetMutationResult {
                success: true,
                tweet_id: Some(tweet_id),
                error: None,
            },
            None => TweetMutationResult {
                success: false,
                tweet_id: None,
                error: Some("Tweet created but no ID returned".to_owned()),
            },
        })
    }

    fn ensure_client_user_id(&self) -> anyhow::Result<()> {
        if let Ok(slot) = self.client_user_id.lock() {
            if slot.is_some() {
                return Ok(());
            }
        }
        let user = self.get_current_user()?;
        if let Ok(mut slot) = self.client_user_id.lock() {
            *slot = Some(user.id);
        }
        Ok(())
    }

    fn try_status_update_fallback(
        &self,
        errors: &[Value],
        variables: &Value,
    ) -> anyhow::Result<Option<TweetMutationResult>> {
        if !errors.iter().any(|error| error.get("code").and_then(Value::as_i64) == Some(226)) {
            return Ok(None);
        }
        let Some(input) = status_update_input_from_create_tweet_variables(variables) else {
            return Ok(None);
        };
        let fallback = self.post_status_update(input)?;
        if fallback.success {
            return Ok(Some(fallback));
        }
        Ok(Some(TweetMutationResult {
            success: false,
            tweet_id: None,
            error: Some(format!(
                "{} | fallback: {}",
                format_errors(errors),
                fallback.error.unwrap_or_else(|| "Unknown error".to_owned())
            )),
        }))
    }

    fn post_status_update(&self, input: StatusUpdateInput) -> anyhow::Result<TweetMutationResult> {
        let mut params = Serializer::new(String::new());
        params.append_pair("status", &input.text);
        if let Some(reply_id) = input.in_reply_to_tweet_id.as_deref() {
            params.append_pair("in_reply_to_status_id", reply_id);
            params.append_pair("auto_populate_reply_metadata", "true");
        }
        if !input.media_ids.is_empty() {
            params.append_pair("media_ids", &input.media_ids.join(","));
        }
        let response = self.send_request(
            "POST",
            TWITTER_STATUS_UPDATE_URL,
            self.headers_form_with_referer("https://x.com/compose/post"),
            Some(params.finish().into_bytes()),
        )?;
        if !response.is_success() {
            return Ok(TweetMutationResult {
                success: false,
                tweet_id: None,
                error: Some(format!(
                    "HTTP {}: {}",
                    response.status,
                    truncate(&response.text(), 200)
                )),
            });
        }
        let data = response.json()?;
        if let Some(errors) = data.get("errors").and_then(Value::as_array) {
            return Ok(TweetMutationResult {
                success: false,
                tweet_id: None,
                error: Some(format_errors(errors)),
            });
        }
        let tweet_id = first_string(&[data.get("id_str"), data.get("id")]);
        Ok(match tweet_id {
            Some(tweet_id) => TweetMutationResult {
                success: true,
                tweet_id: Some(tweet_id),
                error: None,
            },
            None => TweetMutationResult {
                success: false,
                tweet_id: None,
                error: Some("Tweet created but no ID returned".to_owned()),
            },
        })
    }

    fn upload_media_impl(
        &self,
        data: &[u8],
        mime_type: &str,
        category: &str,
        alt: Option<&str>,
    ) -> anyhow::Result<String> {
        let init_body = urlencoded_body(&[
            ("command", "INIT"),
            ("total_bytes", &data.len().to_string()),
            ("media_type", mime_type),
            ("media_category", category),
        ]);
        let init_response = self.send_request(
            "POST",
            TWITTER_UPLOAD_URL,
            self.headers_form(),
            Some(init_body),
        )?;
        if !init_response.is_success() {
            anyhow::bail!(
                "HTTP {}: {}",
                init_response.status,
                truncate(&init_response.text(), 200)
            );
        }
        let init_data = init_response.json()?;
        let media_id = first_string(&[
            init_data.get("media_id_string"),
            init_data.get("media_id"),
        ])
        .context("Media upload INIT did not return media_id")?;

        let chunk_size = 5 * 1024 * 1024;
        let mut segment_index = 0usize;
        for chunk in data.chunks(chunk_size) {
            let (body, boundary) = multipart_form_data(
                &[
                    MultipartField::Text {
                        name: "command",
                        value: "APPEND".to_owned(),
                    },
                    MultipartField::Text {
                        name: "media_id",
                        value: media_id.clone(),
                    },
                    MultipartField::Text {
                        name: "segment_index",
                        value: segment_index.to_string(),
                    },
                    MultipartField::File {
                        name: "media",
                        filename: "media".to_owned(),
                        content_type: mime_type.to_owned(),
                        data: chunk.to_vec(),
                    },
                ],
            );
            let response = self.send_request(
                "POST",
                TWITTER_UPLOAD_URL,
                self.headers_multipart(&boundary),
                Some(body),
            )?;
            if !response.is_success() {
                anyhow::bail!("HTTP {}: {}", response.status, truncate(&response.text(), 200));
            }
            segment_index += 1;
        }

        let finalize_body = urlencoded_body(&[("command", "FINALIZE"), ("media_id", &media_id)]);
        let finalize_response = self.send_request(
            "POST",
            TWITTER_UPLOAD_URL,
            self.headers_form(),
            Some(finalize_body),
        )?;
        if !finalize_response.is_success() {
            anyhow::bail!(
                "HTTP {}: {}",
                finalize_response.status,
                truncate(&finalize_response.text(), 200)
            );
        }
        let finalize_data = finalize_response.json()?;
        maybe_wait_for_media_processing(self, &media_id, finalize_data.get("processing_info"))?;

        if let Some(alt) = alt.filter(|alt| !alt.is_empty() && mime_type.starts_with("image/")) {
            let response = self.send_request(
                "POST",
                TWITTER_MEDIA_METADATA_URL,
                self.headers_json(),
                Some(
                    serde_json::to_vec(&json!({
                        "media_id": media_id,
                        "alt_text": { "text": alt }
                    }))?,
                ),
            )?;
            if !response.is_success() {
                anyhow::bail!("HTTP {}: {}", response.status, truncate(&response.text(), 200));
            }
        }

        Ok(media_id)
    }

    fn get_users_page(
        &self,
        user_id: &str,
        count: usize,
        cursor: Option<String>,
        max_pages: Option<usize>,
        following: bool,
    ) -> anyhow::Result<UsersPage> {
        let operation_name = if following { "Following" } else { "Followers" };
        let query_ids = if following {
            self.following_query_ids()
        } else {
            self.followers_query_ids()
        };
        let rest_action = if following { "friends" } else { "followers" };
        let unlimited = max_pages.is_some();
        let page_size = if unlimited { 20usize } else { count.max(1) };
        let mut seen = std::collections::BTreeSet::new();
        let mut users = Vec::new();
        let mut cursor = cursor;
        let mut next_cursor = None;
        let mut pages_fetched = 0usize;

        while unlimited || users.len() < count {
            let page_count = if unlimited {
                page_size
            } else {
                page_size.min(count.saturating_sub(users.len()))
            };
            let page = self
                .fetch_users_page_graphql(user_id, page_count, cursor.clone(), operation_name, &query_ids)
                .or_else(|_| self.fetch_users_page_rest(user_id, page_count, cursor.clone(), rest_action))?;
            pages_fetched += 1;
            let mut added = 0usize;
            for user in page.users {
                if seen.insert(user.id.clone()) {
                    users.push(user);
                    added += 1;
                    if !unlimited && users.len() >= count {
                        break;
                    }
                }
            }
            if page.next_cursor.is_none()
                || page.next_cursor == cursor
                || added == 0
                || max_pages.map(|max| pages_fetched >= max).unwrap_or(false)
            {
                next_cursor = page.next_cursor;
                break;
            }
            cursor = page.next_cursor.clone();
            next_cursor = cursor.clone();
        }

        Ok(UsersPage { users, next_cursor })
    }

    fn get_lists_for_user(
        &self,
        operation_name: &str,
        query_ids: &[String],
        user_id: &str,
        count: usize,
    ) -> anyhow::Result<Vec<TwitterList>> {
        let features = build_lists_features();
        let variables = json!({
            "userId": user_id,
            "count": count,
            "isListMembershipShown": true,
            "isListMemberTargetUserId": user_id
        });
        let params = encode_params(&[
            ("variables", serde_json::to_string(&variables)?),
            ("features", serde_json::to_string(&features)?),
        ]);

        self.with_refreshed_query_ids_on_error(|| {
            let mut last_error = None;
            for query_id in query_ids {
                let url = format!("{TWITTER_API_BASE}/{query_id}/{operation_name}?{params}");
                let response = self.send_request("GET", &url, self.headers_json(), None);
                let Ok(response) = response else {
                    last_error = Some(response.err().unwrap().to_string());
                    continue;
                };
                if response.status == 404 {
                    last_error = Some("HTTP 404".to_owned());
                    continue;
                }
                if !response.is_success() {
                    last_error = Some(format!(
                        "HTTP {}: {}",
                        response.status,
                        truncate(&response.text(), 200)
                    ));
                    continue;
                }
                let data = response.json().map_err(|error| RefreshableError {
                    message: error.to_string(),
                    needs_refresh: false,
                })?;
                if let Some(errors) = data.get("errors").and_then(Value::as_array) {
                    last_error = Some(format_errors(errors));
                    continue;
                }
                let instructions = data
                    .get("data")
                    .and_then(|value| value.get("user"))
                    .and_then(|value| value.get("result"))
                    .and_then(|value| value.get("timeline"))
                    .and_then(|value| value.get("timeline"))
                    .and_then(|value| value.get("instructions"))
                    .and_then(Value::as_array)
                    .map(Vec::as_slice);
                return Ok(parse_lists_from_instructions(instructions));
            }

            Err(RefreshableError {
                message: last_error.unwrap_or_else(|| format!("Unknown error fetching {operation_name}")),
                needs_refresh: true,
            })
        })
        .map_err(anyhow::Error::msg)
    }

    fn fetch_tweet_timeline_page(
        &self,
        query_ids: &[String],
        operation_name: &str,
        params: &str,
        instructions_path: &[&str],
        include_raw: bool,
        refresh_on_query_error: bool,
    ) -> anyhow::Result<TweetTimelinePage> {
        self.with_refreshed_query_ids_on_error(|| {
            let mut last_error = None;
            let mut needs_refresh = false;

            for query_id in query_ids {
                let url = format!("{TWITTER_API_BASE}/{query_id}/{operation_name}?{params}");
                let response = self.send_request("GET", &url, self.headers_json(), None);
                let Ok(response) = response else {
                    last_error = Some(response.err().unwrap().to_string());
                    continue;
                };
                if response.status == 404 {
                    needs_refresh = true;
                    last_error = Some("HTTP 404".to_owned());
                    continue;
                }
                if !response.is_success() {
                    last_error = Some(format!(
                        "HTTP {}: {}",
                        response.status,
                        truncate(&response.text(), 200)
                    ));
                    continue;
                }

                let data = response.json().map_err(|error| RefreshableError {
                    message: error.to_string(),
                    needs_refresh: false,
                })?;
                let instructions = vget_path(&data, instructions_path)
                    .and_then(Value::as_array)
                    .map(Vec::as_slice);

                if let Some(errors) = data.get("errors").and_then(Value::as_array) {
                    let message = format_errors(errors);
                    let is_refreshable = refresh_on_query_error
                        && (message.contains("Query: Unspecified")
                            || message.contains("GRAPHQL_VALIDATION_FAILED")
                            || message.contains("must be defined"));
                    if instructions.is_none() || is_refreshable {
                        last_error = Some(message);
                        needs_refresh |= is_refreshable;
                        continue;
                    }
                }

                return Ok(TweetTimelinePage {
                    tweets: parse_tweets_from_instructions(instructions, self.quote_depth, include_raw),
                    cursor: extract_cursor_from_instructions(instructions, "Bottom"),
                });
            }

            Err(RefreshableError {
                message: last_error.unwrap_or_else(|| format!("Unknown error fetching {operation_name}")),
                needs_refresh,
            })
        })
        .map_err(anyhow::Error::msg)
    }

    fn fetch_tweet_timeline_page_with_retry(
        &self,
        query_ids: &[String],
        operation_name: &str,
        params: &str,
        instructions_path: &[&str],
        include_raw: bool,
        refresh_on_query_error: bool,
    ) -> anyhow::Result<TweetTimelinePage> {
        self.with_refreshed_query_ids_on_error(|| {
            let mut last_error = None;
            let mut needs_refresh = false;

            for query_id in query_ids {
                let url = format!("{TWITTER_API_BASE}/{query_id}/{operation_name}?{params}");
                let response = self.fetch_with_retry("GET", &url, self.headers_json(), None);
                let Ok(response) = response else {
                    last_error = Some(response.err().unwrap().to_string());
                    continue;
                };
                if response.status == 404 {
                    needs_refresh = true;
                    last_error = Some("HTTP 404".to_owned());
                    continue;
                }
                if !response.is_success() {
                    last_error = Some(format!(
                        "HTTP {}: {}",
                        response.status,
                        truncate(&response.text(), 200)
                    ));
                    continue;
                }

                let data = response.json().map_err(|error| RefreshableError {
                    message: error.to_string(),
                    needs_refresh: false,
                })?;
                let instructions = vget_path(&data, instructions_path)
                    .and_then(Value::as_array)
                    .map(Vec::as_slice);

                if let Some(errors) = data.get("errors").and_then(Value::as_array) {
                    let message = format_errors(errors);
                    let is_refreshable = refresh_on_query_error
                        && (message.contains("Query: Unspecified")
                            || message.contains("GRAPHQL_VALIDATION_FAILED")
                            || message.contains("must be defined"));
                    if instructions.is_none() || is_refreshable {
                        last_error = Some(message);
                        needs_refresh |= is_refreshable;
                        continue;
                    }
                }

                return Ok(TweetTimelinePage {
                    tweets: parse_tweets_from_instructions(instructions, self.quote_depth, include_raw),
                    cursor: extract_cursor_from_instructions(instructions, "Bottom"),
                });
            }

            Err(RefreshableError {
                message: last_error.unwrap_or_else(|| format!("Unknown error fetching {operation_name}")),
                needs_refresh,
            })
        })
        .map_err(anyhow::Error::msg)
    }

    fn fetch_bookmark_folder_page(
        &self,
        folder_id: &str,
        page_count: usize,
        cursor: Option<String>,
        features: &Value,
        include_raw: bool,
    ) -> anyhow::Result<TweetTimelinePage> {
        self.with_refreshed_query_ids_on_error(|| {
            let mut last_error = None;

            for query_id in self.bookmark_folder_query_ids() {
                let variables_with_count = json!({
                    "bookmark_collection_id": folder_id,
                    "includePromotedContent": true,
                    "count": page_count,
                    "cursor": cursor
                });
                let variables_without_count = json!({
                    "bookmark_collection_id": folder_id,
                    "includePromotedContent": true,
                    "cursor": cursor
                });

                for variables in [variables_with_count, variables_without_count] {
                    let params = encode_params(&[
                        ("variables", serde_json::to_string(&variables).map_err(|error| RefreshableError {
                            message: error.to_string(),
                            needs_refresh: false,
                        })?),
                        ("features", serde_json::to_string(features).map_err(|error| RefreshableError {
                            message: error.to_string(),
                            needs_refresh: false,
                        })?),
                    ]);
                    let url =
                        format!("{TWITTER_API_BASE}/{query_id}/BookmarkFolderTimeline?{params}");
                    let response =
                        self.fetch_with_retry("GET", &url, self.headers_json(), None);
                    let Ok(response) = response else {
                        last_error = Some(response.err().unwrap().to_string());
                        continue;
                    };
                    if response.status == 404 {
                        last_error = Some("HTTP 404".to_owned());
                        continue;
                    }
                    if !response.is_success() {
                        last_error = Some(format!(
                            "HTTP {}: {}",
                            response.status,
                            truncate(&response.text(), 200)
                        ));
                        continue;
                    }

                    let data = response.json().map_err(|error| RefreshableError {
                        message: error.to_string(),
                        needs_refresh: false,
                    })?;
                    let instructions = data
                        .get("data")
                        .and_then(|value| value.get("bookmark_collection_timeline"))
                        .and_then(|value| value.get("timeline"))
                        .and_then(|value| value.get("instructions"))
                        .and_then(Value::as_array)
                        .map(Vec::as_slice);

                    if let Some(errors) = data.get("errors").and_then(Value::as_array) {
                        let message = format_errors(errors);
                        if instructions.is_none() {
                            if message.contains("Variable \"$count\"") {
                                continue;
                            }
                            if message.contains("Variable \"$cursor\"") && cursor.is_some() {
                                return Err(RefreshableError {
                                    message: "Bookmark folder pagination rejected the cursor parameter".to_owned(),
                                    needs_refresh: false,
                                });
                            }
                            last_error = Some(message);
                            continue;
                        }
                    }

                    return Ok(TweetTimelinePage {
                        tweets: parse_tweets_from_instructions(instructions, self.quote_depth, include_raw),
                        cursor: extract_cursor_from_instructions(instructions, "Bottom"),
                    });
                }
            }

            Err(RefreshableError {
                message: last_error.unwrap_or_else(|| "Unknown error fetching bookmark folder".to_owned()),
                needs_refresh: true,
            })
        })
        .map_err(anyhow::Error::msg)
    }

    fn fetch_news_tab(
        &self,
        tab_name: &str,
        timeline_id: &str,
        max_count: usize,
        ai_only: bool,
        include_raw: bool,
    ) -> anyhow::Result<Vec<NewsItem>> {
        let features = build_explore_features();
        let query_ids = self.generic_timeline_query_ids();
        let variables = json!({
            "timelineId": timeline_id,
            "count": max_count.saturating_mul(2),
            "includePromotedContent": false
        });
        let params = encode_params(&[
            ("variables", serde_json::to_string(&variables)?),
            ("features", serde_json::to_string(&features)?),
        ]);
        let data = self.fetch_graphql_json(
            &query_ids,
            "GenericTimelineById",
            "GET",
            &params,
            None,
            true,
        )?;

        Ok(parse_news_items_from_timeline(
            data.get("timeline").and_then(|value| value.get("timeline")),
            tab_name,
            max_count,
            ai_only,
            include_raw,
        ))
    }

    fn fetch_users_page_graphql(
        &self,
        user_id: &str,
        count: usize,
        cursor: Option<String>,
        operation_name: &str,
        query_ids: &[String],
    ) -> anyhow::Result<UsersPage> {
        let features = build_following_features();
        let variables = json!({
            "userId": user_id,
            "count": count,
            "includePromotedContent": false,
            "cursor": cursor
        });
        let params = encode_params(&[
            ("variables", serde_json::to_string(&variables)?),
            ("features", serde_json::to_string(&features)?),
        ]);

        self.with_refreshed_query_ids_on_error(|| {
            let mut last_error = None;
            for query_id in query_ids {
                let url = format!("{TWITTER_API_BASE}/{query_id}/{operation_name}?{params}");
                let response = self.send_request("GET", &url, self.headers_json(), None);
                let Ok(response) = response else {
                    last_error = Some(response.err().unwrap().to_string());
                    continue;
                };
                if response.status == 404 {
                    last_error = Some("HTTP 404".to_owned());
                    continue;
                }
                if !response.is_success() {
                    last_error = Some(format!(
                        "HTTP {}: {}",
                        response.status,
                        truncate(&response.text(), 200)
                    ));
                    continue;
                }
                let data = response.json().map_err(|error| RefreshableError {
                    message: error.to_string(),
                    needs_refresh: false,
                })?;
                if let Some(errors) = data.get("errors").and_then(Value::as_array) {
                    last_error = Some(format_errors(errors));
                    continue;
                }
                let instructions = data
                    .get("data")
                    .and_then(|value| value.get("user"))
                    .and_then(|value| value.get("result"))
                    .and_then(|value| value.get("timeline"))
                    .and_then(|value| value.get("timeline"))
                    .and_then(|value| value.get("instructions"))
                    .and_then(Value::as_array)
                    .map(Vec::as_slice);
                return Ok(UsersPage {
                    users: parse_users_from_instructions(instructions),
                    next_cursor: extract_cursor_from_instructions(instructions, "Bottom"),
                });
            }

            Err(RefreshableError {
                message: last_error.unwrap_or_else(|| format!("Unknown error fetching {operation_name}")),
                needs_refresh: true,
            })
        })
        .map_err(anyhow::Error::msg)
    }

    fn fetch_users_page_rest(
        &self,
        user_id: &str,
        count: usize,
        cursor: Option<String>,
        action: &str,
    ) -> anyhow::Result<UsersPage> {
        let mut params = Serializer::new(String::new());
        params.append_pair("user_id", user_id);
        params.append_pair("count", &count.to_string());
        params.append_pair("skip_status", "true");
        params.append_pair("include_user_entities", "false");
        if let Some(cursor) = cursor.as_deref() {
            params.append_pair("cursor", cursor);
        }
        let params = params.finish();
        let urls = [
            format!("https://x.com/i/api/1.1/{action}/list.json?{params}"),
            format!("https://api.twitter.com/1.1/{action}/list.json?{params}"),
        ];
        let mut last_error = None;

        for url in urls {
            let response = self.send_request("GET", &url, self.headers_json(), None);
            let Ok(response) = response else {
                last_error = Some(response.err().unwrap().to_string());
                continue;
            };
            if !response.is_success() {
                last_error = Some(format!(
                    "HTTP {}: {}",
                    response.status,
                    truncate(&response.text(), 200)
                ));
                continue;
            }
            let data = response.json()?;
            let next_cursor = data
                .get("next_cursor_str")
                .and_then(Value::as_str)
                .filter(|value| *value != "0" && !value.is_empty())
                .map(ToOwned::to_owned);
            return Ok(UsersPage {
                users: parse_users_from_rest_response(data.get("users").and_then(Value::as_array)),
                next_cursor,
            });
        }

        anyhow::bail!(
            "{}",
            last_error.unwrap_or_else(|| format!("Unknown error fetching {action}"))
        )
    }

    fn client_user_id_value(&self) -> anyhow::Result<String> {
        self.ensure_client_user_id()?;
        self.client_user_id
            .lock()
            .ok()
            .and_then(|slot| slot.clone())
            .context("Could not determine client user id")
    }

    fn follow_via_rest(&self, user_id: &str, action: &str) -> anyhow::Result<FollowMutationResult> {
        let urls = [
            format!("https://x.com/i/api/1.1/friendships/{action}.json"),
            format!("https://api.twitter.com/1.1/friendships/{action}.json"),
        ];
        let body = urlencoded_body(&[
            ("user_id", user_id),
            ("skip_status", "true"),
        ]);
        let mut last_error = None;

        for url in urls {
            let response = self.send_request("POST", &url, self.headers_form(), Some(body.clone()));
            let Ok(response) = response else {
                last_error = Some(response.err().unwrap().to_string());
                continue;
            };

            if !response.is_success() {
                let text = response.text();
                if let Ok(data) = serde_json::from_str::<Value>(&text) {
                    if let Some(error) = data
                        .get("errors")
                        .and_then(Value::as_array)
                        .and_then(|errors| errors.first())
                    {
                        match error.get("code").and_then(Value::as_i64) {
                            Some(160) => {
                                return Ok(FollowMutationResult {
                                    success: true,
                                    user_id: None,
                                    username: None,
                                    error: None,
                                })
                            }
                            Some(162) => {
                                return Ok(FollowMutationResult {
                                    success: false,
                                    user_id: None,
                                    username: None,
                                    error: Some(
                                        "You have been blocked from following this account"
                                            .to_owned(),
                                    ),
                                })
                            }
                            Some(108) => {
                                return Ok(FollowMutationResult {
                                    success: false,
                                    user_id: None,
                                    username: None,
                                    error: Some("User not found".to_owned()),
                                })
                            }
                            _ => {
                                let message = error
                                    .get("message")
                                    .and_then(Value::as_str)
                                    .unwrap_or("Unknown error");
                                let code = error
                                    .get("code")
                                    .and_then(Value::as_i64)
                                    .map(|code| format!(" (code {code})"))
                                    .unwrap_or_default();
                                last_error = Some(format!("{message}{code}"));
                                continue;
                            }
                        }
                    }
                }
                last_error = Some(format!(
                    "HTTP {}: {}",
                    response.status,
                    truncate(&text, 200)
                ));
                continue;
            }

            let data = response.json()?;
            if let Some(errors) = data.get("errors").and_then(Value::as_array) {
                last_error = Some(format_errors(errors));
                continue;
            }
            let result = FollowMutationResult {
                success: true,
                user_id: first_string(&[data.get("id_str"), data.get("id")]),
                username: first_string(&[data.get("screen_name")]),
                error: None,
            };
            return Ok(result);
        }

        Ok(FollowMutationResult {
            success: false,
            user_id: None,
            username: None,
            error: Some(last_error.unwrap_or_else(|| format!("Unknown error during {action}"))),
        })
    }

    fn follow_via_graphql(&self, user_id: &str, follow: bool) -> anyhow::Result<FollowMutationResult> {
        let operation_name = if follow {
            "CreateFriendship"
        } else {
            "DestroyFriendship"
        };
        let variables = json!({ "user_id": user_id });

        let try_once = || -> anyhow::Result<(FollowMutationResult, bool)> {
            let mut had_404 = false;
            let mut last_error = None;
            for query_id in self.follow_query_ids(follow) {
                let url = format!("{TWITTER_API_BASE}/{query_id}/{operation_name}");
                let response = self.send_request(
                    "POST",
                    &url,
                    self.headers_json(),
                    Some(serde_json::to_vec(&json!({
                        "variables": variables,
                        "queryId": query_id
                    }))?),
                )?;
                if response.status == 404 {
                    had_404 = true;
                    last_error = Some("HTTP 404".to_owned());
                    continue;
                }
                if !response.is_success() {
                    last_error = Some(format!(
                        "HTTP {}: {}",
                        response.status,
                        truncate(&response.text(), 200)
                    ));
                    continue;
                }
                let data = response.json()?;
                if let Some(errors) = data.get("errors").and_then(Value::as_array) {
                    last_error = Some(format_errors(errors));
                    continue;
                }
                let result = data
                    .get("data")
                    .and_then(|value| value.get("user"))
                    .and_then(|value| value.get("result"));
                return Ok((
                    FollowMutationResult {
                        success: true,
                        user_id: result
                            .and_then(|value| value.get("rest_id"))
                            .and_then(Value::as_str)
                            .map(ToOwned::to_owned),
                        username: result
                            .and_then(|value| value.get("legacy"))
                            .and_then(|value| value.get("screen_name"))
                            .and_then(Value::as_str)
                            .map(ToOwned::to_owned),
                        error: None,
                    },
                    had_404,
                ));
            }
            Ok((
                FollowMutationResult {
                    success: false,
                    user_id: None,
                    username: None,
                    error: Some(last_error.unwrap_or_else(|| format!("Unknown error during {operation_name}"))),
                },
                had_404,
            ))
        };

        let (first_attempt, had_404) = try_once()?;
        if first_attempt.success {
            return Ok(first_attempt);
        }
        if had_404 {
            let _ = self.refresh_query_ids();
            let (second_attempt, _) = try_once()?;
            return Ok(second_attempt);
        }
        Ok(first_attempt)
    }

    pub fn get_home_timeline(&self, count: usize, include_raw: bool) -> anyhow::Result<Vec<TweetData>> {
        self.fetch_home_timeline("HomeTimeline", count, include_raw)
    }

    pub fn get_home_latest_timeline(
        &self,
        count: usize,
        include_raw: bool,
    ) -> anyhow::Result<Vec<TweetData>> {
        self.fetch_home_timeline("HomeLatestTimeline", count, include_raw)
    }

    pub fn search(
        &self,
        query: &str,
        count: usize,
        include_raw: bool,
        cursor: Option<String>,
        max_pages: Option<usize>,
    ) -> anyhow::Result<TweetsPage> {
        let page_size = 20usize;
        let mut seen = std::collections::BTreeSet::new();
        let mut tweets = Vec::new();
        let mut cursor = cursor;
        let mut next_cursor = None;
        let mut pages_fetched = 0usize;
        let unlimited = max_pages.is_some();
        while unlimited || tweets.len() < count {
            let page_count = if unlimited {
                page_size
            } else {
                page_size.min(count.saturating_sub(tweets.len()))
            };
            let page = self.fetch_search_page(query, page_count, cursor.clone(), include_raw)?;
            pages_fetched += 1;
            let mut added = 0usize;
            for tweet in page.tweets {
                if seen.insert(tweet.id.clone()) {
                    tweets.push(tweet);
                    added += 1;
                    if !unlimited && tweets.len() >= count {
                        break;
                    }
                }
            }
            if page.next_cursor.is_none()
                || page.next_cursor == cursor
                || added == 0
                || max_pages.map(|max| pages_fetched >= max).unwrap_or(false)
            {
                next_cursor = page.next_cursor;
                break;
            }
            cursor = page.next_cursor.clone();
            next_cursor = cursor.clone();
        }
        Ok(TweetsPage { tweets, next_cursor })
    }

    pub fn get_tweet(&self, tweet_id: &str, include_raw: bool) -> anyhow::Result<TweetData> {
        let data = self.fetch_tweet_detail(tweet_id, None)?;
        let tweet_result = data
            .get("tweetResult")
            .and_then(|value| value.get("result"))
            .cloned()
            .or_else(|| {
                find_tweet_in_instructions(
                    data.get("threaded_conversation_with_injections_v2")
                        .and_then(|value| value.get("instructions"))
                        .and_then(Value::as_array)
                        .map(Vec::as_slice),
                    tweet_id,
                )
            })
            .context("Tweet not found in response")?;
        map_tweet_result(&tweet_result, self.quote_depth, include_raw).context("Tweet not found in response")
    }

    pub fn get_replies(
        &self,
        tweet_id: &str,
        include_raw: bool,
        cursor: Option<String>,
        max_pages: Option<usize>,
        page_delay: Duration,
    ) -> anyhow::Result<TweetsPage> {
        self.get_thread_like(tweet_id, include_raw, cursor, max_pages, page_delay, true)
    }

    pub fn get_thread(
        &self,
        tweet_id: &str,
        include_raw: bool,
        cursor: Option<String>,
        max_pages: Option<usize>,
        page_delay: Duration,
    ) -> anyhow::Result<TweetsPage> {
        self.get_thread_like(tweet_id, include_raw, cursor, max_pages, page_delay, false)
    }

    pub fn get_user_id_by_username(&self, username: &str) -> anyhow::Result<(String, String, Option<String>)> {
        let clean = normalize_handle(username).context("Invalid username")?;
        let query_ids = [
            "xc8f1g7BYqr6VTzTbvNlGw",
            "qW5u-DAuXpMEG0zA1F7UGQ",
            "sLVLhk0bGj3MVFEKTdax1w",
        ];
        let variables = json!({
            "screen_name": clean,
            "withSafetyModeUserFields": true
        });
        let features = json!({
            "hidden_profile_subscriptions_enabled": true,
            "hidden_profile_likes_enabled": true,
            "rweb_tipjar_consumption_enabled": true,
            "responsive_web_graphql_exclude_directive_enabled": true,
            "verified_phone_label_enabled": false,
            "subscriptions_verification_info_is_identity_verified_enabled": true,
            "subscriptions_verification_info_verified_since_enabled": true,
            "highlights_tweets_tab_ui_enabled": true,
            "responsive_web_twitter_article_notes_tab_enabled": true,
            "subscriptions_feature_can_gift_premium": true,
            "creator_subscriptions_tweet_preview_api_enabled": true,
            "responsive_web_graphql_skip_user_profile_image_extensions_enabled": false,
            "responsive_web_graphql_timeline_navigation_enabled": true,
            "blue_business_profile_image_shape_enabled": true
        });
        let field_toggles = json!({ "withAuxiliaryUserLabels": false });
        let params = encode_params(&[
            ("variables", serde_json::to_string(&variables)?),
            ("features", serde_json::to_string(&features)?),
            ("fieldToggles", serde_json::to_string(&field_toggles)?),
        ]);
        for query_id in query_ids {
            let url = format!("{TWITTER_API_BASE}/{query_id}/UserByScreenName?{params}");
            let response = self.send_request("GET", &url, self.headers_json(), None)?;
            if response.status == 404 {
                continue;
            }
            if !response.is_success() {
                continue;
            }
            let data = response.json()?;
            if data
                .get("data")
                .and_then(|value| value.get("user"))
                .and_then(|value| value.get("result"))
                .and_then(|value| value.get("__typename"))
                .and_then(Value::as_str)
                == Some("UserUnavailable")
            {
                anyhow::bail!("User @{clean} not found or unavailable");
            }
            let user_result = data
                .get("data")
                .and_then(|value| value.get("user"))
                .and_then(|value| value.get("result"));
            let user_id = user_result
                .and_then(|value| value.get("rest_id"))
                .and_then(Value::as_str)
                .map(ToOwned::to_owned);
            let username = first_string(&[
                user_result.and_then(|value| value.get("legacy")).and_then(|value| value.get("screen_name")),
                user_result.and_then(|value| value.get("core")).and_then(|value| value.get("screen_name")),
            ]);
            let name = first_string(&[
                user_result.and_then(|value| value.get("legacy")).and_then(|value| value.get("name")),
                user_result.and_then(|value| value.get("core")).and_then(|value| value.get("name")),
            ]);
            if let (Some(user_id), Some(username)) = (user_id, username) {
                return Ok((user_id, username, name));
            }
        }

        let urls = [
            format!("https://x.com/i/api/1.1/users/show.json?screen_name={}", urlencoding(&clean)),
            format!("https://api.twitter.com/1.1/users/show.json?screen_name={}", urlencoding(&clean)),
        ];
        for url in urls {
            let response = self.send_request("GET", &url, self.headers_json(), None)?;
            if response.status == 404 {
                anyhow::bail!("User @{clean} not found");
            }
            if !response.is_success() {
                continue;
            }
            let data = response.json()?;
            let user_id = first_string(&[data.get("id_str"), data.get("id")]).context("Could not parse user ID from response")?;
            return Ok((
                user_id,
                first_string(&[data.get("screen_name")]).unwrap_or(clean),
                first_string(&[data.get("name")]),
            ));
        }
        anyhow::bail!("Unknown error looking up user")
    }

    pub fn get_user_tweets(
        &self,
        user_id: &str,
        count: usize,
        include_raw: bool,
        cursor: Option<String>,
        max_pages: Option<usize>,
        page_delay: Duration,
    ) -> anyhow::Result<TweetsPage> {
        let page_size = 20usize;
        let hard_max_pages = 10usize;
        let computed_max_pages = ((count + page_size - 1) / page_size).max(1);
        let effective_max_pages = max_pages.unwrap_or(computed_max_pages).min(hard_max_pages);
        let mut seen = std::collections::BTreeSet::new();
        let mut tweets = Vec::new();
        let mut cursor = cursor;
        let mut next_cursor = None;
        let mut pages_fetched = 0usize;

        while tweets.len() < count {
            if pages_fetched > 0 && page_delay > Duration::ZERO {
                thread::sleep(page_delay);
            }
            let remaining = count.saturating_sub(tweets.len());
            let page = self.fetch_user_tweets_page(
                user_id,
                remaining.min(page_size),
                cursor.clone(),
                include_raw,
            )?;
            pages_fetched += 1;
            let mut added = 0usize;
            for tweet in page.tweets {
                if seen.insert(tweet.id.clone()) {
                    tweets.push(tweet);
                    added += 1;
                    if tweets.len() >= count {
                        break;
                    }
                }
            }
            if tweets.len() >= count {
                break;
            }
            if page.next_cursor.is_none()
                || page.next_cursor == cursor
                || added == 0
                || pages_fetched >= effective_max_pages
            {
                next_cursor = page.next_cursor;
                break;
            }
            cursor = page.next_cursor.clone();
            next_cursor = cursor.clone();
        }
        Ok(TweetsPage { tweets, next_cursor })
    }

    fn fetch_home_timeline(
        &self,
        operation: &str,
        count: usize,
        include_raw: bool,
    ) -> anyhow::Result<Vec<TweetData>> {
        let query_ids = if operation == "HomeTimeline" {
            self.home_timeline_query_ids()
        } else {
            self.home_latest_timeline_query_ids()
        };
        let mut tweets = Vec::new();
        let mut seen = std::collections::BTreeSet::new();
        let mut cursor = None;
        while tweets.len() < count {
            let page_count = 20usize.min(count.saturating_sub(tweets.len()));
            let variables = json!({
                "count": page_count,
                "includePromotedContent": true,
                "latestControlAvailable": true,
                "requestContext": "launch",
                "withCommunity": true,
                "cursor": cursor
            });
            let features = build_home_timeline_features();
            let params = encode_params(&[
                ("variables", serde_json::to_string(&variables)?),
                ("features", serde_json::to_string(&features)?),
            ]);
            let data = self.fetch_graphql_json(&query_ids, operation, "GET", &params, None, true)?;
            let instructions = data
                .get("home")
                .and_then(|value| value.get("home_timeline_urt"))
                .and_then(|value| value.get("instructions"))
                .and_then(Value::as_array)
                .map(Vec::as_slice);
            let page_tweets = parse_tweets_from_instructions(instructions, self.quote_depth, include_raw);
            let next_cursor = extract_cursor_from_instructions(instructions, "Bottom");
            let mut added = 0usize;
            for tweet in page_tweets {
                if seen.insert(tweet.id.clone()) {
                    tweets.push(tweet);
                    added += 1;
                    if tweets.len() >= count {
                        break;
                    }
                }
            }
            if tweets.len() >= count {
                break;
            }
            if next_cursor.is_none() || next_cursor == cursor || added == 0 {
                break;
            }
            cursor = next_cursor;
        }
        Ok(tweets)
    }

    fn fetch_search_page(
        &self,
        query: &str,
        count: usize,
        cursor: Option<String>,
        include_raw: bool,
    ) -> anyhow::Result<TweetsPage> {
        let query_ids = self.search_timeline_query_ids();
        let variables = json!({
            "rawQuery": query,
            "count": count,
            "querySource": "typed_query",
            "product": "Latest",
            "cursor": cursor
        });
        let params = encode_params(&[("variables", serde_json::to_string(&variables)?)]);
        let body = json!({
            "features": build_search_features(),
            "queryId": query_ids[0]
        });
        let data = self.fetch_graphql_json(
            &query_ids,
            "SearchTimeline",
            "POST",
            &params,
            Some(serde_json::to_vec(&body)?),
            true,
        )?;
        let instructions = data
            .get("search_by_raw_query")
            .and_then(|value| value.get("search_timeline"))
            .and_then(|value| value.get("timeline"))
            .and_then(|value| value.get("instructions"))
            .and_then(Value::as_array)
            .map(Vec::as_slice);
        Ok(TweetsPage {
            tweets: parse_tweets_from_instructions(instructions, self.quote_depth, include_raw),
            next_cursor: extract_cursor_from_instructions(instructions, "Bottom"),
        })
    }

    fn fetch_tweet_detail(&self, tweet_id: &str, cursor: Option<String>) -> anyhow::Result<Value> {
        let variables = json!({
            "focalTweetId": tweet_id,
            "with_rux_injections": false,
            "rankingMode": "Relevance",
            "includePromotedContent": true,
            "withCommunity": true,
            "withQuickPromoteEligibilityTweetFields": true,
            "withBirdwatchNotes": true,
            "withVoice": true,
            "cursor": cursor
        });
        let features_value = {
            let mut features_map = build_tweet_detail_features()
                .as_object()
                .cloned()
                .unwrap_or_default();
            for (key, value) in [
                ("articles_preview_enabled", Value::Bool(true)),
                ("articles_rest_api_enabled", Value::Bool(true)),
                (
                    "responsive_web_graphql_skip_user_profile_image_extensions_enabled",
                    Value::Bool(false),
                ),
                (
                    "creator_subscriptions_tweet_preview_api_enabled",
                    Value::Bool(true),
                ),
                (
                    "graphql_is_translatable_rweb_tweet_is_translatable_enabled",
                    Value::Bool(true),
                ),
                ("view_counts_everywhere_api_enabled", Value::Bool(true)),
                ("longform_notetweets_consumption_enabled", Value::Bool(true)),
                (
                    "responsive_web_twitter_article_tweet_consumption_enabled",
                    Value::Bool(true),
                ),
                ("freedom_of_speech_not_reach_fetch_enabled", Value::Bool(true)),
                ("standardized_nudges_misinfo", Value::Bool(true)),
                (
                    "tweet_with_visibility_results_prefer_gql_limited_actions_policy_enabled",
                    Value::Bool(true),
                ),
                ("rweb_video_timestamps_enabled", Value::Bool(true)),
            ] {
                features_map.insert(key.to_owned(), value);
            }
            Value::Object(features_map)
        };
        let field_toggles_value = {
            let mut field_toggles = build_article_field_toggles()
                .as_object()
                .cloned()
                .unwrap_or_default();
            field_toggles.insert("withArticleRichContentState".to_owned(), Value::Bool(true));
            Value::Object(field_toggles)
        };
        let params = encode_params(&[
            ("variables", serde_json::to_string(&variables)?),
            ("features", serde_json::to_string(&features_value)?),
            ("fieldToggles", serde_json::to_string(&field_toggles_value)?),
        ]);
        let mut refreshed = false;

        loop {
            for query_id in self.tweet_detail_query_ids() {
                let get_url = format!("{TWITTER_API_BASE}/{query_id}/TweetDetail?{params}");
                let get_response = self.send_request("GET", &get_url, self.headers_json(), None)?;
                if get_response.status != 404 {
                    return self.parse_graphql_detail_response(get_response);
                }

                let post_url = format!("{TWITTER_API_BASE}/{query_id}/TweetDetail");
                let post_body = serde_json::to_vec(&json!({
                    "variables": variables,
                    "features": features_value,
                    "queryId": query_id
                }))?;
                let post_response =
                    self.send_request("POST", &post_url, self.headers_json(), Some(post_body))?;
                if post_response.status != 404 {
                    return self.parse_graphql_detail_response(post_response);
                }
            }

            if refreshed {
                anyhow::bail!("Unable to resolve a working query id for TweetDetail");
            }

            let _ = self.refresh_query_ids();
            refreshed = true;
        }
    }

    fn parse_graphql_detail_response(
        &self,
        response: crate::transport::HttpResponse,
    ) -> anyhow::Result<Value> {
        if !response.is_success() {
            anyhow::bail!("HTTP {}: {}", response.status, truncate(&response.text(), 200));
        }
        let data = response.json()?;
        if let Some(errors) = data.get("errors").and_then(Value::as_array) {
            let has_usable_data = data
                .get("data")
                .and_then(|data| {
                    data.get("tweetResult")
                        .and_then(|value| value.get("result"))
                        .or_else(|| {
                            data.get("threaded_conversation_with_injections_v2")
                                .and_then(|value| value.get("instructions"))
                        })
                })
                .is_some();
            if !has_usable_data {
                anyhow::bail!(
                    "{}",
                    errors
                        .iter()
                        .filter_map(|error| error.get("message").and_then(Value::as_str))
                        .collect::<Vec<_>>()
                        .join(", ")
                );
            }
        }
        Ok(data.get("data").cloned().unwrap_or(Value::Null))
    }

    fn fetch_user_tweets_page(
        &self,
        user_id: &str,
        count: usize,
        cursor: Option<String>,
        include_raw: bool,
    ) -> anyhow::Result<TweetsPage> {
        let query_ids = self.user_tweets_query_ids();
        let variables = json!({
            "userId": user_id,
            "count": count,
            "includePromotedContent": false,
            "withQuickPromoteEligibilityTweetFields": true,
            "withVoice": true,
            "cursor": cursor
        });
        let field_toggles = json!({ "withArticlePlainText": false });
        let params = encode_params(&[
            ("variables", serde_json::to_string(&variables)?),
            ("features", serde_json::to_string(&build_user_tweets_features())?),
            ("fieldToggles", serde_json::to_string(&field_toggles)?),
        ]);
        let data = self.fetch_graphql_json(&query_ids, "UserTweets", "GET", &params, None, true)?;
        let instructions = data
            .get("user")
            .and_then(|value| value.get("result"))
            .and_then(|value| value.get("timeline"))
            .and_then(|value| value.get("timeline"))
            .and_then(|value| value.get("instructions"))
            .and_then(Value::as_array)
            .map(Vec::as_slice);
        Ok(TweetsPage {
            tweets: parse_tweets_from_instructions(instructions, self.quote_depth, include_raw),
            next_cursor: extract_cursor_from_instructions(instructions, "Bottom"),
        })
    }

    fn get_thread_like(
        &self,
        tweet_id: &str,
        include_raw: bool,
        mut cursor: Option<String>,
        max_pages: Option<usize>,
        page_delay: Duration,
        replies_only: bool,
    ) -> anyhow::Result<TweetsPage> {
        let mut seen = std::collections::BTreeSet::new();
        let mut tweets = Vec::new();
        let mut root_id = None;
        let mut pages = 0usize;
        let next_cursor = loop {
            let data = self.fetch_tweet_detail(tweet_id, cursor.clone())?;
            let instructions = data
                .get("threaded_conversation_with_injections_v2")
                .and_then(|value| value.get("instructions"))
                .and_then(Value::as_array)
                .map(Vec::as_slice);
            let parsed = parse_tweets_from_instructions(instructions, self.quote_depth, include_raw);
            if root_id.is_none() {
                root_id = parsed
                    .iter()
                    .find(|tweet| tweet.id == tweet_id)
                    .and_then(|tweet| tweet.conversation_id.clone())
                    .or_else(|| Some(tweet_id.to_owned()));
            }
            let filtered = if replies_only {
                parsed
                    .into_iter()
                    .filter(|tweet| tweet.in_reply_to_status_id.as_deref() == Some(tweet_id))
                    .collect::<Vec<_>>()
            } else {
                let root_id = root_id.clone().unwrap_or_else(|| tweet_id.to_owned());
                let mut filtered = parsed
                    .into_iter()
                    .filter(|tweet| tweet.conversation_id.as_deref() == Some(root_id.as_str()))
                    .collect::<Vec<_>>();
                filtered.sort_by_key(|tweet| tweet.created_at.as_ref().map(|created_at| created_at.to_owned()));
                filtered
            };
            let mut added = 0usize;
            for tweet in filtered {
                if seen.insert(tweet.id.clone()) {
                    tweets.push(tweet);
                    added += 1;
                }
            }
            pages += 1;
            let next_cursor = extract_cursor_from_instructions(instructions, "Bottom");
            if next_cursor.is_none()
                || next_cursor == cursor
                || added == 0
                || max_pages.map(|max| pages >= max).unwrap_or(false)
            {
                break next_cursor;
            }
            cursor = next_cursor.clone();
            if page_delay > Duration::ZERO {
                thread::sleep(page_delay);
            }
        };
        Ok(TweetsPage { tweets, next_cursor })
    }

    fn fetch_graphql_json(
        &self,
        query_ids: &[String],
        operation_name: &str,
        method: &str,
        params: &str,
        body: Option<Vec<u8>>,
        refresh_on_404: bool,
    ) -> anyhow::Result<Value> {
        let mut refreshed = false;
        loop {
            let mut saw_refreshable_miss = false;
            for query_id in query_ids {
                let url = format!("{TWITTER_API_BASE}/{query_id}/{operation_name}?{params}");
                let response = self.send_request(method, &url, self.headers_json(), body.clone())?;
                if response.status == 404 {
                    saw_refreshable_miss = true;
                    continue;
                }

                if !response.is_success() {
                    let body = response.text();
                    let should_refresh = response.status == 400
                        && (body.contains("GRAPHQL_VALIDATION_FAILED")
                            || body.contains("rawQuery")
                            || body.contains("query: unspecified")
                            || body.contains("must be defined"));
                    if should_refresh {
                        saw_refreshable_miss = true;
                    } else {
                        anyhow::bail!("HTTP {}: {}", response.status, truncate(&body, 200));
                    }
                    continue;
                }

                let data = response.json()?;
                if let Some(errors) = data.get("errors").and_then(Value::as_array) {
                    let message = errors
                        .iter()
                        .filter_map(|error| error.get("message").and_then(Value::as_str))
                        .collect::<Vec<_>>()
                        .join(", ");
                    if message.contains("query: unspecified")
                        || message.contains("GRAPHQL_VALIDATION_FAILED")
                        || message.contains("must be defined")
                    {
                        saw_refreshable_miss = true;
                        continue;
                    }
                    anyhow::bail!("{message}");
                }
                return Ok(data.get("data").cloned().unwrap_or(Value::Null));
            }

            if !saw_refreshable_miss || !refresh_on_404 || refreshed {
                anyhow::bail!("Unable to resolve a working query id for {operation_name}");
            }

            let _ = self.refresh_query_ids();
            refreshed = true;
        }
    }

    fn send_request(
        &self,
        method: &str,
        url: &str,
        mut headers: Vec<(String, String)>,
        body: Option<Vec<u8>>,
    ) -> anyhow::Result<crate::transport::HttpResponse> {
        if !headers.iter().any(|(name, _)| name.eq_ignore_ascii_case("x-client-transaction-id")) {
            let transaction_id = self
                .transaction_ids
                .generate(&self.transport, &self.user_agent, method, url)
                .unwrap_or_else(|_| random_transaction_id());
            headers.push(("x-client-transaction-id".into(), transaction_id));
        }
        self.transport.send(&HttpRequest {
            method: method.to_owned(),
            url: url.to_owned(),
            headers,
            body,
            timeout: self.timeout,
        })
    }

    fn fetch_with_retry(
        &self,
        method: &str,
        url: &str,
        headers: Vec<(String, String)>,
        body: Option<Vec<u8>>,
    ) -> anyhow::Result<crate::transport::HttpResponse> {
        let max_retries = 2usize;
        let base_delay = Duration::from_millis(500);
        for attempt in 0..=max_retries {
            let response = self.send_request(method, url, headers.clone(), body.clone())?;
            if !matches!(response.status, 429 | 500 | 502 | 503 | 504) || attempt == max_retries {
                return Ok(response);
            }

            let retry_after = response
                .headers
                .get("retry-after")
                .and_then(|value| value.parse::<u64>().ok())
                .map(Duration::from_secs);
            let jitter = Duration::from_millis(rand::random::<u64>() % 500);
            let backoff = retry_after.unwrap_or_else(|| base_delay * 2u32.pow(attempt as u32) + jitter);
            thread::sleep(backoff);
        }
        self.send_request(method, url, headers, body)
    }

    fn headers_json(&self) -> Vec<(String, String)> {
        let mut headers = self.base_headers();
        headers.push(("content-type".into(), "application/json".into()));
        headers
    }

    fn headers_json_with_referer(&self, referer: &str) -> Vec<(String, String)> {
        let mut headers = self.headers_json();
        override_header(&mut headers, "referer", referer.to_owned());
        headers
    }

    fn headers_form(&self) -> Vec<(String, String)> {
        let mut headers = self.base_headers();
        headers.push((
            "content-type".into(),
            "application/x-www-form-urlencoded".into(),
        ));
        headers
    }

    fn headers_form_with_referer(&self, referer: &str) -> Vec<(String, String)> {
        let mut headers = self.headers_form();
        override_header(&mut headers, "referer", referer.to_owned());
        headers
    }

    fn headers_multipart(&self, boundary: &str) -> Vec<(String, String)> {
        let mut headers = self.base_headers();
        headers.push((
            "content-type".into(),
            format!("multipart/form-data; boundary={boundary}"),
        ));
        headers
    }

    fn base_headers(&self) -> Vec<(String, String)> {
        let chrome_version = chrome_version_from_user_agent(&self.user_agent);
        let mut headers = vec![
            ("accept".into(), "*/*".into()),
            ("accept-language".into(), "en-US,en;q=0.9".into()),
            ("authorization".into(), format!("Bearer {BEARER_TOKEN}")),
            ("x-csrf-token".into(), self.cookies.ct0.clone().unwrap_or_default()),
            ("x-twitter-auth-type".into(), "OAuth2Session".into()),
            ("x-twitter-active-user".into(), "yes".into()),
            ("x-twitter-client-language".into(), "en".into()),
            ("x-client-uuid".into(), self.client_uuid.clone()),
            ("x-twitter-client-deviceid".into(), self.client_device_id.clone()),
            ("cookie".into(), self.cookie_header()),
            ("user-agent".into(), self.user_agent.clone()),
            (
                "sec-ch-ua".into(),
                format!(
                    "\"Chromium\";v=\"{chrome_version}\", \"Not(A:Brand\";v=\"99\", \"Google Chrome\";v=\"{chrome_version}\""
                ),
            ),
            ("sec-ch-ua-mobile".into(), "?0".into()),
            ("sec-ch-ua-platform".into(), "\"macOS\"".into()),
            ("sec-fetch-dest".into(), "empty".into()),
            ("sec-fetch-mode".into(), "cors".into()),
            ("sec-fetch-site".into(), "same-origin".into()),
            ("priority".into(), "u=1, i".into()),
            ("origin".into(), "https://x.com".into()),
            ("referer".into(), "https://x.com/".into()),
        ];
        if let Ok(slot) = self.client_user_id.lock() {
            if let Some(user_id) = slot.as_ref() {
                headers.push(("x-twitter-client-user-id".into(), user_id.clone()));
            }
        }
        headers
    }

    fn cookie_header(&self) -> String {
        self.cookies.cookie_header.clone().unwrap_or_else(|| {
            format!(
                "auth_token={}; ct0={}",
                self.cookies.auth_token.as_deref().unwrap_or_default(),
                self.cookies.ct0.as_deref().unwrap_or_default()
            )
        })
    }

    fn query_id(&self, operation: &str) -> String {
        self.query_ids
            .get_query_id(operation)
            .unwrap_or_default()
    }

    fn home_timeline_query_ids(&self) -> Vec<String> {
        unique(vec![self.query_id("HomeTimeline"), "edseUwk9sP5Phz__9TIRnA".into()])
    }

    fn home_latest_timeline_query_ids(&self) -> Vec<String> {
        unique(vec![self.query_id("HomeLatestTimeline"), "iOEZpOdfekFsxSlPQCQtPg".into()])
    }

    fn search_timeline_query_ids(&self) -> Vec<String> {
        unique(vec![
            self.query_id("SearchTimeline"),
            "M1jEez78PEfVfbQLvlWMvQ".into(),
            "5h0kNbk3ii97rmfY6CdgAA".into(),
            "Tp1sewRU1AsZpBWhqCZicQ".into(),
        ])
    }

    fn tweet_detail_query_ids(&self) -> Vec<String> {
        unique(vec![
            self.query_id("TweetDetail"),
            "97JF30KziU00483E_8elBA".into(),
            "aFvUsJm2c-oDkJV75blV6g".into(),
        ])
    }

    fn user_tweets_query_ids(&self) -> Vec<String> {
        unique(vec![self.query_id("UserTweets"), "Wms1GvIiHXAPBaCr9KblaA".into()])
    }

    fn follow_query_ids(&self, follow: bool) -> Vec<String> {
        if follow {
            return unique(vec![
                self.query_id("CreateFriendship"),
                "8h9JVdV8dlSyqyRDJEPCsA".into(),
                "OPwKc1HXnBT_bWXfAlo-9g".into(),
            ]);
        }
        unique(vec![
            self.query_id("DestroyFriendship"),
            "ppXWuagMNXgvzx6WoXBW0Q".into(),
            "8h9JVdV8dlSyqyRDJEPCsA".into(),
        ])
    }

    fn likes_query_ids(&self) -> Vec<String> {
        unique(vec![self.query_id("Likes"), "JR2gceKucIKcVNB_9JkhsA".into()])
    }

    fn bookmarks_query_ids(&self) -> Vec<String> {
        unique(vec![
            self.query_id("Bookmarks"),
            "RV1g3b8n_SGOHwkqKYSCFw".into(),
            "tmd4ifV8RHltzn8ymGg1aw".into(),
        ])
    }

    fn bookmark_folder_query_ids(&self) -> Vec<String> {
        unique(vec![
            self.query_id("BookmarkFolderTimeline"),
            "KJIQpsvxrTfRIlbaRIySHQ".into(),
        ])
    }

    fn following_query_ids(&self) -> Vec<String> {
        unique(vec![self.query_id("Following"), "BEkNpEt5pNETESoqMsTEGA".into()])
    }

    fn followers_query_ids(&self) -> Vec<String> {
        unique(vec![self.query_id("Followers"), "kuFUYP9eV1FPoEy4N-pi7w".into()])
    }

    fn list_ownerships_query_ids(&self) -> Vec<String> {
        unique(vec![self.query_id("ListOwnerships"), "wQcOSjSQ8NtgxIwvYl1lMg".into()])
    }

    fn list_timeline_query_ids(&self) -> Vec<String> {
        unique(vec![
            self.query_id("ListLatestTweetsTimeline"),
            "2TemLyqrMpTeAmysdbnVqw".into(),
        ])
    }

    fn about_account_query_ids(&self) -> Vec<String> {
        unique(vec![self.query_id("AboutAccountQuery"), "zs_jFPFT78rBpXv9Z3U2YQ".into()])
    }

    fn generic_timeline_query_ids(&self) -> Vec<String> {
        unique(vec![
            self.query_id("GenericTimelineById"),
            "uGSr7alSjR9v6QJAIaqSKQ".into(),
            "QkTEwlbNN7EY8LsXdXwDLw".into(),
        ])
    }

    fn with_refreshed_query_ids_on_error<T, F>(&self, mut callback: F) -> Result<T, String>
    where
        F: FnMut() -> Result<T, RefreshableError>,
    {
        match callback() {
            Ok(value) => Ok(value),
            Err(first_error) if first_error.needs_refresh => {
                let _ = self.refresh_query_ids();
                callback().map_err(|second_error| second_error.message)
            }
            Err(error) => Err(error.message),
        }
    }
}

fn chrome_version_from_user_agent(user_agent: &str) -> String {
    CHROME_VERSION_RE
        .captures(user_agent)
        .and_then(|captures| captures.get(1).map(|value| value.as_str().to_owned()))
        .unwrap_or_else(|| "131".to_owned())
}

fn random_transaction_id() -> String {
    let mut bytes = [0u8; 16];
    rand::rng().fill(&mut bytes);
    hex::encode(bytes)
}

#[derive(Debug, Clone)]
struct TweetTimelinePage {
    tweets: Vec<TweetData>,
    cursor: Option<String>,
}

#[derive(Debug, Clone)]
struct RefreshableError {
    message: String,
    needs_refresh: bool,
}

fn parse_users_from_rest_response(users: Option<&Vec<Value>>) -> Vec<TwitterUser> {
    users
        .into_iter()
        .flatten()
        .filter_map(|user| {
            let id = first_string(&[user.get("id_str"), user.get("id")])?;
            let username = first_string(&[user.get("screen_name")])?;
            Some(TwitterUser {
                id,
                username: username.clone(),
                name: first_string(&[user.get("name")]).unwrap_or(username),
                description: first_string(&[user.get("description")]),
                followers_count: user.get("followers_count").and_then(Value::as_u64),
                following_count: user.get("friends_count").and_then(Value::as_u64),
                is_blue_verified: user.get("verified").and_then(Value::as_bool),
                profile_image_url: first_string(&[user.get("profile_image_url_https")]),
                created_at: first_string(&[user.get("created_at")]),
            })
        })
        .collect()
}

fn vget_path<'a>(value: &'a Value, path: &[&str]) -> Option<&'a Value> {
    let mut current = value;
    for segment in path {
        current = current.get(*segment)?;
    }
    Some(current)
}

fn first_string(values: &[Option<&Value>]) -> Option<String> {
    values
        .iter()
        .flatten()
        .find_map(|value| {
            value
                .as_str()
                .map(ToOwned::to_owned)
                .or_else(|| value.as_u64().map(|value| value.to_string()))
        })
}

fn encode_params(entries: &[(&str, String)]) -> String {
    let mut serializer = Serializer::new(String::new());
    for (key, value) in entries {
        serializer.append_pair(key, value);
    }
    serializer.finish()
}

fn normalize_handle(value: &str) -> Option<String> {
    let trimmed = value.trim().trim_start_matches('@');
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_owned())
    }
}

fn truncate(value: &str, max_len: usize) -> String {
    value.chars().take(max_len).collect()
}

fn unique(values: Vec<String>) -> Vec<String> {
    let mut seen = std::collections::BTreeSet::new();
    let mut unique = Vec::new();
    for value in values {
        if !value.is_empty() && seen.insert(value.clone()) {
            unique.push(value);
        }
    }
    unique
}

fn urlencoding(value: &str) -> String {
    let mut serializer = Serializer::new(String::new());
    serializer.append_pair("value", value);
    serializer
        .finish()
        .trim_start_matches("value=")
        .to_owned()
}

#[derive(Debug, Clone)]
struct StatusUpdateInput {
    text: String,
    in_reply_to_tweet_id: Option<String>,
    media_ids: Vec<String>,
}

fn status_update_input_from_create_tweet_variables(variables: &Value) -> Option<StatusUpdateInput> {
    let text = variables
        .get("tweet_text")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())?
        .to_owned();
    let in_reply_to_tweet_id = variables
        .get("reply")
        .and_then(|value| value.get("in_reply_to_tweet_id"))
        .and_then(Value::as_str)
        .map(ToOwned::to_owned);
    let media_ids = variables
        .get("media")
        .and_then(|value| value.get("media_entities"))
        .and_then(Value::as_array)
        .map(|entities| {
            entities
                .iter()
                .filter_map(|entity| entity.get("media_id"))
                .filter_map(|value| {
                    value
                        .as_str()
                        .map(ToOwned::to_owned)
                        .or_else(|| value.as_u64().map(|value| value.to_string()))
                })
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    Some(StatusUpdateInput {
        text,
        in_reply_to_tweet_id,
        media_ids,
    })
}

fn format_errors(errors: &[Value]) -> String {
    errors
        .iter()
        .filter_map(|error| {
            let message = error.get("message").and_then(Value::as_str)?;
            Some(match error.get("code").and_then(Value::as_i64) {
                Some(code) => format!("{message} ({code})"),
                None => message.to_owned(),
            })
        })
        .collect::<Vec<_>>()
        .join(", ")
}

fn parse_bookmark_mutation_response(
    response: crate::transport::HttpResponse,
) -> anyhow::Result<BookmarkMutationResult> {
    if !response.is_success() {
        return Ok(BookmarkMutationResult {
            success: false,
            error: Some(format!(
                "HTTP {}: {}",
                response.status,
                truncate(&response.text(), 200)
            )),
        });
    }

    let data = response.json()?;
    if let Some(errors) = data.get("errors").and_then(Value::as_array) {
        return Ok(BookmarkMutationResult {
            success: false,
            error: Some(format_errors(errors)),
        });
    }

    Ok(BookmarkMutationResult {
        success: true,
        error: None,
    })
}

fn explore_timeline_id(tab: &str) -> Option<&'static str> {
    match tab {
        EXPLORE_TAB_FOR_YOU => Some(EXPLORE_TIMELINE_FOR_YOU),
        EXPLORE_TAB_TRENDING => Some(EXPLORE_TIMELINE_TRENDING),
        EXPLORE_TAB_NEWS => Some(EXPLORE_TIMELINE_NEWS),
        EXPLORE_TAB_SPORTS => Some(EXPLORE_TIMELINE_SPORTS),
        EXPLORE_TAB_ENTERTAINMENT => Some(EXPLORE_TIMELINE_ENTERTAINMENT),
        _ => None,
    }
}

fn parse_news_items_from_timeline(
    timeline: Option<&Value>,
    source: &str,
    max_count: usize,
    ai_only: bool,
    include_raw: bool,
) -> Vec<NewsItem> {
    let instructions = timeline
        .and_then(|value| value.get("instructions"))
        .and_then(Value::as_array);
    let mut items = Vec::new();
    let mut seen = std::collections::BTreeSet::new();

    for instruction in instructions.unwrap_or(&Vec::new()) {
        let entries = instruction
            .get("entries")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_else(|| {
                instruction
                    .get("entry")
                    .cloned()
                    .map(|entry| vec![entry])
                    .unwrap_or_default()
            });
        for entry in entries {
            if items.len() >= max_count {
                break;
            }
            let entry_id = entry.get("entryId").and_then(Value::as_str);
            let Some(content) = entry.get("content") else {
                continue;
            };
            if let Some(item_content) = content.get("itemContent") {
                if let Some(item) =
                    parse_news_item_from_content(item_content, entry_id, source, &mut seen, ai_only, include_raw)
                {
                    items.push(item);
                }
            }
            if let Some(content_items) = content.get("items").and_then(Value::as_array) {
                for item in content_items {
                    if items.len() >= max_count {
                        break;
                    }
                    let item_content = item
                        .get("itemContent")
                        .or_else(|| item.get("item").and_then(|value| value.get("itemContent")));
                    if let Some(item_content) = item_content {
                        if let Some(item) = parse_news_item_from_content(
                            item_content,
                            entry_id,
                            source,
                            &mut seen,
                            ai_only,
                            include_raw,
                        ) {
                            items.push(item);
                        }
                    }
                }
            }
        }
        if items.len() >= max_count {
            break;
        }
    }

    items
}

fn parse_news_item_from_content(
    item_content: &Value,
    entry_id: Option<&str>,
    source: &str,
    seen_headlines: &mut std::collections::BTreeSet<String>,
    ai_only: bool,
    include_raw: bool,
) -> Option<NewsItem> {
    let headline = first_string(&[item_content.get("name"), item_content.get("title")])?;
    if !seen_headlines.insert(headline.clone()) {
        return None;
    }

    let social_context = item_content
        .get("social_context")
        .and_then(|value| value.get("text"))
        .and_then(Value::as_str)
        .unwrap_or_default();
    let has_news_category = social_context.contains("News") || social_context.contains("hours ago");
    let is_full_sentence = headline.split_whitespace().count() >= 5;
    let is_explicit_ai = item_content
        .get("is_ai_trend")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let is_ai_news = is_explicit_ai || (is_full_sentence && has_news_category);
    if ai_only && !is_ai_news {
        return None;
    }

    let trend_metadata = item_content.get("trend_metadata");
    let url = first_string(&[
        item_content
            .get("trend_url")
            .and_then(|value| value.get("url")),
        trend_metadata
            .and_then(|value| value.get("url"))
            .and_then(|value| value.get("url")),
    ]);

    let mut category = Some("Trending".to_owned());
    let mut time_ago = None;
    let mut post_count = None;

    for part in social_context.split('·').map(str::trim).filter(|part| !part.is_empty()) {
        if part.contains("ago") {
            time_ago = Some(part.to_owned());
        } else if let Some(count) = parse_post_count(part) {
            post_count = Some(count);
        } else {
            category = Some(part.to_owned());
        }
    }

    if post_count.is_none() {
        post_count = trend_metadata
            .and_then(|value| value.get("meta_description"))
            .and_then(Value::as_str)
            .and_then(parse_post_count);
    }

    if let Some(domain_context) = trend_metadata
        .and_then(|value| value.get("domain_context"))
        .and_then(Value::as_str)
    {
        if matches!(category.as_deref(), Some("Trending") | Some("News")) {
            category = Some(domain_context.to_owned());
        }
    }
    if is_ai_news {
        category = Some(match category {
            Some(value) => format!("AI · {value}"),
            None => "AI".to_owned(),
        });
    }

    Some(NewsItem {
        id: url
            .clone()
            .or_else(|| entry_id.map(|entry_id| format!("{entry_id}-{headline}")))
            .unwrap_or_else(|| format!("{source}-{headline}")),
        headline,
        category,
        time_ago,
        post_count,
        description: first_string(&[item_content.get("description")]),
        url,
        tweets: None,
        raw: include_raw.then(|| item_content.clone()),
    })
}

fn parse_post_count(value: &str) -> Option<u64> {
    let captures = POST_COUNT_RE.captures(value)?;
    let number = captures.get(1)?.as_str().parse::<f64>().ok()?;
    let multiplier = match captures
        .get(2)
        .map(|value| value.as_str().to_ascii_uppercase())
        .as_deref()
    {
        Some("K") => 1_000f64,
        Some("M") => 1_000_000f64,
        Some("B") => 1_000_000_000f64,
        _ => 1f64,
    };
    Some((number * multiplier).round() as u64)
}

fn media_entities(media_ids: Option<&[String]>) -> Vec<Value> {
    media_ids
        .unwrap_or(&[])
        .iter()
        .map(|id| {
            json!({
                "media_id": id,
                "tagged_users": []
            })
        })
        .collect()
}

fn media_category_for_mime(mime_type: &str) -> Option<&'static str> {
    if mime_type.starts_with("image/") {
        return Some(if mime_type == "image/gif" {
            "tweet_gif"
        } else {
            "tweet_image"
        });
    }
    if mime_type.starts_with("video/") {
        return Some("tweet_video");
    }
    None
}

fn urlencoded_body(entries: &[(&str, &str)]) -> Vec<u8> {
    let mut serializer = Serializer::new(String::new());
    for (key, value) in entries {
        serializer.append_pair(key, value);
    }
    serializer.finish().into_bytes()
}

fn override_header(headers: &mut Vec<(String, String)>, name: &str, value: String) {
    if let Some(existing) = headers
        .iter_mut()
        .find(|(header_name, _)| header_name.eq_ignore_ascii_case(name))
    {
        existing.1 = value;
    } else {
        headers.push((name.to_owned(), value));
    }
}

#[derive(Debug, Clone)]
enum MultipartField {
    Text {
        name: &'static str,
        value: String,
    },
    File {
        name: &'static str,
        filename: String,
        content_type: String,
        data: Vec<u8>,
    },
}

fn multipart_form_data(fields: &[MultipartField]) -> (Vec<u8>, String) {
    let mut boundary_bytes = [0u8; 12];
    rand::rng().fill(&mut boundary_bytes);
    let boundary = format!("bird-{}", hex::encode(boundary_bytes));
    let mut body = Vec::new();
    for field in fields {
        body.extend_from_slice(format!("--{boundary}\r\n").as_bytes());
        match field {
            MultipartField::Text { name, value } => {
                body.extend_from_slice(
                    format!("Content-Disposition: form-data; name=\"{name}\"\r\n\r\n").as_bytes(),
                );
                body.extend_from_slice(value.as_bytes());
                body.extend_from_slice(b"\r\n");
            }
            MultipartField::File {
                name,
                filename,
                content_type,
                data,
            } => {
                body.extend_from_slice(
                    format!(
                        "Content-Disposition: form-data; name=\"{name}\"; filename=\"{filename}\"\r\n"
                    )
                    .as_bytes(),
                );
                body.extend_from_slice(
                    format!("Content-Type: {content_type}\r\n\r\n").as_bytes(),
                );
                body.extend_from_slice(data);
                body.extend_from_slice(b"\r\n");
            }
        }
    }
    body.extend_from_slice(format!("--{boundary}--\r\n").as_bytes());
    (body, boundary)
}

fn maybe_wait_for_media_processing(
    client: &TwitterClient,
    media_id: &str,
    processing_info: Option<&Value>,
) -> anyhow::Result<()> {
    let Some(mut state) = processing_info
        .and_then(parse_processing_info)
        .filter(|state| state.state != "succeeded")
    else {
        return Ok(());
    };

    let mut attempts = 0usize;
    while attempts < 20 {
        if state.state == "failed" {
            anyhow::bail!(
                "{}",
                state
                    .error
                    .unwrap_or_else(|| "Media processing failed".to_owned())
            );
        }
        thread::sleep(Duration::from_secs(state.check_after_secs.max(1)));
        let status_url = format!(
            "{TWITTER_UPLOAD_URL}?{}",
            Serializer::new(String::new())
                .append_pair("command", "STATUS")
                .append_pair("media_id", media_id)
                .finish()
        );
        let response = client.send_request("GET", &status_url, client.base_headers(), None)?;
        if !response.is_success() {
            anyhow::bail!("HTTP {}: {}", response.status, truncate(&response.text(), 200));
        }
        let data = response.json()?;
        let Some(next_state) = data.get("processing_info").and_then(parse_processing_info) else {
            break;
        };
        if next_state.state == "succeeded" {
            break;
        }
        state = next_state;
        attempts += 1;
    }
    Ok(())
}

#[derive(Debug, Clone)]
struct MediaProcessingInfo {
    state: String,
    check_after_secs: u64,
    error: Option<String>,
}

fn parse_processing_info(value: &Value) -> Option<MediaProcessingInfo> {
    Some(MediaProcessingInfo {
        state: value.get("state")?.as_str()?.to_owned(),
        check_after_secs: value
            .get("check_after_secs")
            .and_then(Value::as_u64)
            .unwrap_or(2),
        error: value
            .get("error")
            .and_then(|error| {
                error
                    .get("message")
                    .and_then(Value::as_str)
                    .or_else(|| error.get("name").and_then(Value::as_str))
            })
            .map(ToOwned::to_owned),
    })
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::{
        media_category_for_mime, multipart_form_data, status_update_input_from_create_tweet_variables,
        MultipartField,
    };

    #[test]
    fn media_category_maps_expected_types() {
        assert_eq!(media_category_for_mime("image/png"), Some("tweet_image"));
        assert_eq!(media_category_for_mime("image/gif"), Some("tweet_gif"));
        assert_eq!(media_category_for_mime("video/mp4"), Some("tweet_video"));
        assert_eq!(media_category_for_mime("application/pdf"), None);
    }

    #[test]
    fn status_update_input_extracts_reply_and_media_ids() {
        let variables = json!({
            "tweet_text": "hello",
            "reply": { "in_reply_to_tweet_id": "123" },
            "media": {
                "media_entities": [
                    { "media_id": "m1", "tagged_users": [] },
                    { "media_id": 42, "tagged_users": [] }
                ]
            }
        });

        let input = status_update_input_from_create_tweet_variables(&variables).expect("input");

        assert_eq!(input.text, "hello");
        assert_eq!(input.in_reply_to_tweet_id.as_deref(), Some("123"));
        assert_eq!(input.media_ids, vec!["m1".to_owned(), "42".to_owned()]);
    }

    #[test]
    fn multipart_encoder_contains_named_parts() {
        let (body, boundary) = multipart_form_data(&[
            MultipartField::Text {
                name: "command",
                value: "APPEND".to_owned(),
            },
            MultipartField::File {
                name: "media",
                filename: "media".to_owned(),
                content_type: "image/png".to_owned(),
                data: b"abc".to_vec(),
            },
        ]);
        let text = String::from_utf8(body).expect("utf8");

        assert!(text.contains(&format!("--{boundary}")));
        assert!(text.contains("name=\"command\""));
        assert!(text.contains("APPEND"));
        assert!(text.contains("name=\"media\"; filename=\"media\""));
        assert!(text.contains("Content-Type: image/png"));
    }
}
