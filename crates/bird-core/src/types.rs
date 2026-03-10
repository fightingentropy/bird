use std::collections::BTreeMap;
use std::path::PathBuf;
use std::time::Duration;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum CookieSource {
    Safari,
    Chrome,
    Firefox,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct TwitterCookies {
    pub auth_token: Option<String>,
    pub ct0: Option<String>,
    pub cookie_header: Option<String>,
    pub source: Option<String>,
}

#[derive(Debug, Clone, Default)]
pub struct ResolveCredentialsOptions {
    pub auth_token: Option<String>,
    pub ct0: Option<String>,
    pub cookie_source: Vec<CookieSource>,
    pub chrome_profile: Option<String>,
    pub firefox_profile: Option<String>,
    pub cookie_timeout: Option<Duration>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ResolvedCredentials {
    pub cookies: TwitterCookies,
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct TwitterClientOptions {
    pub cookies: TwitterCookies,
    pub user_agent: Option<String>,
    pub timeout: Option<Duration>,
    pub quote_depth: Option<usize>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TweetAuthor {
    pub username: String,
    pub name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TweetArticle {
    pub title: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub preview_text: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TweetMedia {
    #[serde(rename = "type")]
    pub media_type: String,
    pub url: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub width: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub height: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub preview_url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub video_url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub duration_ms: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TweetData {
    pub id: String,
    pub text: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub created_at: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reply_count: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub retweet_count: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub like_count: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub conversation_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub in_reply_to_status_id: Option<String>,
    pub author: TweetAuthor,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub author_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub quoted_tweet: Option<Box<TweetData>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub media: Option<Vec<TweetMedia>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub article: Option<TweetArticle>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub is_thread: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub thread_position: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub has_self_replies: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub thread_root_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub raw: Option<serde_json::Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CurrentUser {
    pub id: String,
    pub username: String,
    pub name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TwitterUser {
    pub id: String,
    pub username: String,
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub followers_count: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub following_count: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub is_blue_verified: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub profile_image_url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub created_at: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TweetsPage {
    pub tweets: Vec<TweetData>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next_cursor: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UsersPage {
    pub users: Vec<TwitterUser>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next_cursor: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AboutProfile {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub account_based_in: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub created_country_accurate: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub location_accurate: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub learn_more_url: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TwitterListOwner {
    pub id: String,
    pub username: String,
    pub name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TwitterList {
    pub id: String,
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub member_count: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub subscriber_count: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub is_private: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub created_at: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub owner: Option<TwitterListOwner>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NewsItem {
    pub id: String,
    pub headline: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub category: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub time_ago: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub post_count: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tweets: Option<Vec<TweetData>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub raw: Option<serde_json::Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct QueryIdSnapshot {
    pub cached: bool,
    pub cache_path: PathBuf,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fetched_at: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub is_fresh: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub age_ms: Option<u64>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub ids: BTreeMap<String, String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub discovery: Option<serde_json::Value>,
    pub features_path: PathBuf,
    pub features: serde_json::Value,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct TweetMutationResult {
    pub success: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tweet_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct MediaUploadResult {
    pub success: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub media_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct BookmarkMutationResult {
    pub success: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct FollowMutationResult {
    pub success: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub user_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub username: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}
