#![recursion_limit = "256"]

mod client;
mod credentials;
mod features;
mod parser;
mod query_ids;
mod security;
mod transaction_id;
mod transport;
mod types;

pub use client::TwitterClient;
pub use credentials::{
    build_cookie_header_from_cookies, default_cookie_cache_path, default_user_agent,
    resolve_credentials, verify_cookies,
};
pub use features::{
    build_article_features, build_article_field_toggles, build_bookmarks_features,
    build_explore_features, build_following_features, build_home_timeline_features,
    build_likes_features, build_lists_features, build_search_features, build_tweet_create_features,
    build_tweet_detail_features, build_user_tweets_features, features_path, features_snapshot,
    refresh_features_cache,
};
pub use query_ids::{
    RuntimeQueryIdStore, default_cache_path as default_query_ids_cache_path, fallback_query_ids,
    target_query_id_operations,
};
pub use security::{redact_sensitive_text, redacted_marker, register_secret_for_redaction};
pub use transaction_id::RuntimeTransactionIdStore;
pub use transport::{CurlTransport, HttpRequest, HttpResponse, HttpTransport, TransportInfo};
pub use types::{
    AboutProfile, BookmarkMutationResult, CookieSource, CurrentUser, FollowMutationResult,
    MediaUploadResult, NewsItem, QueryIdSnapshot, ResolveCredentialsOptions, ResolvedCredentials,
    TweetArticle, TweetAuthor, TweetData, TweetMedia, TweetMutationResult, TweetsPage,
    TwitterClientOptions, TwitterCookies, TwitterList, TwitterListOwner, TwitterUser, UsersPage,
};
