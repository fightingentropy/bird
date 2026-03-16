use std::collections::{BTreeSet, HashMap};
use std::env;
use std::ffi::OsString;
use std::fs;
use std::path::{Path, PathBuf};
use std::process;
use std::time::Duration;

use bird_core::{
    AboutProfile, CurrentUser, CurlTransport, NewsItem, QueryIdSnapshot,
    ResolveCredentialsOptions, RuntimeQueryIdStore, TweetData, TwitterClient,
    TransportInfo, TwitterClientOptions, TwitterCookies, TwitterList, TwitterUser, UsersPage,
    build_cookie_header_from_cookies,
    refresh_features_cache, resolve_credentials, target_query_id_operations,
};
use clap::{ArgAction, Parser, Subcommand, ValueEnum};
use serde::Deserialize;
use serde_json::json;

const KNOWN_COMMANDS: &[&str] = &[
    "check",
    "transport",
    "whoami",
    "query-ids",
    "tweet",
    "reply",
    "unbookmark",
    "follow",
    "unfollow",
    "likes",
    "following",
    "followers",
    "about",
    "bookmarks",
    "lists",
    "list-timeline",
    "news",
    "trending",
    "home",
    "read",
    "replies",
    "thread",
    "search",
    "mentions",
    "user-tweets",
    "help",
];

#[derive(Debug)]
struct CliError {
    code: i32,
    message: String,
}

impl CliError {
    fn usage(message: impl Into<String>) -> Self {
        Self {
            code: 2,
            message: message.into(),
        }
    }

    fn runtime(message: impl Into<String>) -> Self {
        Self {
            code: 1,
            message: message.into(),
        }
    }
}

impl From<anyhow::Error> for CliError {
    fn from(value: anyhow::Error) -> Self {
        Self::runtime(format!("{value:#}"))
    }
}

type CliResult<T> = Result<T, CliError>;

#[derive(Debug, Clone, Copy, ValueEnum, Deserialize)]
#[serde(rename_all = "lowercase")]
enum CookieSourceArg {
    Safari,
    Chrome,
    Firefox,
}

impl From<CookieSourceArg> for bird_core::CookieSource {
    fn from(value: CookieSourceArg) -> Self {
        match value {
            CookieSourceArg::Safari => Self::Safari,
            CookieSourceArg::Chrome => Self::Chrome,
            CookieSourceArg::Firefox => Self::Firefox,
        }
    }
}

#[derive(Debug, Clone, Parser)]
#[command(
    name = "bird",
    about = "Fast X CLI for reading timelines and tweets",
    version
)]
struct Cli {
    #[command(flatten)]
    global: GlobalOptions,
    #[command(subcommand)]
    command: Commands,
}

#[derive(Debug, Clone, Parser, Default)]
struct GlobalOptions {
    #[arg(long, global = true)]
    auth_token: Option<String>,
    #[arg(long, global = true)]
    ct0: Option<String>,
    #[arg(long = "chrome-profile", global = true)]
    chrome_profile: Option<String>,
    #[arg(long = "chrome-profile-dir", global = true)]
    chrome_profile_dir: Option<String>,
    #[arg(long = "firefox-profile", global = true)]
    firefox_profile: Option<String>,
    #[arg(long = "cookie-timeout", global = true)]
    cookie_timeout: Option<u64>,
    #[arg(
        long = "cookie-source",
        value_enum,
        action = ArgAction::Append,
        global = true
    )]
    cookie_source: Vec<CookieSourceArg>,
    #[arg(long = "media", action = ArgAction::Append, global = true)]
    media: Vec<String>,
    #[arg(long = "alt", action = ArgAction::Append, global = true)]
    alt: Vec<String>,
    #[arg(long, global = true)]
    timeout: Option<u64>,
    #[arg(long = "quote-depth", global = true)]
    quote_depth: Option<usize>,
    #[arg(long, action = ArgAction::SetTrue, global = true)]
    plain: bool,
    #[arg(long = "no-emoji", action = ArgAction::SetTrue, global = true)]
    no_emoji: bool,
    #[arg(long = "no-color", action = ArgAction::SetTrue, global = true)]
    no_color: bool,
}

#[derive(Debug, Clone, Subcommand)]
enum Commands {
    Check,
    Transport {
        #[arg(long)]
        json: bool,
    },
    Whoami,
    Tweet {
        text: String,
    },
    Reply {
        tweet_id_or_url: String,
        text: String,
    },
    Unbookmark {
        #[arg(required = true)]
        tweet_id_or_url: Vec<String>,
    },
    Follow {
        username_or_id: String,
    },
    Unfollow {
        username_or_id: String,
    },
    Likes {
        #[arg(short = 'n', long, default_value_t = 20)]
        count: usize,
        #[arg(long)]
        all: bool,
        #[arg(long = "max-pages")]
        max_pages: Option<usize>,
        #[arg(long)]
        cursor: Option<String>,
        #[arg(long)]
        json: bool,
        #[arg(long = "json-full")]
        json_full: bool,
    },
    Following {
        #[arg(long = "user")]
        user_id: Option<String>,
        #[arg(short = 'n', long, default_value_t = 20)]
        count: usize,
        #[arg(long)]
        cursor: Option<String>,
        #[arg(long)]
        all: bool,
        #[arg(long = "max-pages")]
        max_pages: Option<usize>,
        #[arg(long)]
        json: bool,
    },
    Followers {
        #[arg(long = "user")]
        user_id: Option<String>,
        #[arg(short = 'n', long, default_value_t = 20)]
        count: usize,
        #[arg(long)]
        cursor: Option<String>,
        #[arg(long)]
        all: bool,
        #[arg(long = "max-pages")]
        max_pages: Option<usize>,
        #[arg(long)]
        json: bool,
    },
    About {
        username: String,
        #[arg(long)]
        json: bool,
    },
    Bookmarks {
        #[arg(short = 'n', long, default_value_t = 20)]
        count: usize,
        #[arg(long = "folder-id")]
        folder_id: Option<String>,
        #[arg(long)]
        all: bool,
        #[arg(long = "max-pages")]
        max_pages: Option<usize>,
        #[arg(long)]
        cursor: Option<String>,
        #[arg(long = "expand-root-only")]
        expand_root_only: bool,
        #[arg(long = "author-chain")]
        author_chain: bool,
        #[arg(long = "author-only")]
        author_only: bool,
        #[arg(long = "full-chain-only")]
        full_chain_only: bool,
        #[arg(long = "include-ancestor-branches")]
        include_ancestor_branches: bool,
        #[arg(long = "include-parent")]
        include_parent: bool,
        #[arg(long = "thread-meta")]
        thread_meta: bool,
        #[arg(long = "sort-chronological")]
        sort_chronological: bool,
        #[arg(long)]
        json: bool,
        #[arg(long = "json-full")]
        json_full: bool,
    },
    Lists {
        #[arg(short = 'n', long, default_value_t = 100)]
        count: usize,
        #[arg(long)]
        json: bool,
    },
    #[command(name = "list-timeline")]
    ListTimeline {
        list_id_or_url: String,
        #[arg(short = 'n', long, default_value_t = 20)]
        count: usize,
        #[arg(long)]
        all: bool,
        #[arg(long = "max-pages")]
        max_pages: Option<usize>,
        #[arg(long)]
        cursor: Option<String>,
        #[arg(long)]
        json: bool,
        #[arg(long = "json-full")]
        json_full: bool,
    },
    #[command(alias = "trending")]
    News {
        #[arg(short = 'n', long, default_value_t = 10)]
        count: usize,
        #[arg(long = "ai-only")]
        ai_only: bool,
        #[arg(long = "with-tweets")]
        with_tweets: bool,
        #[arg(long = "tweets-per-item", default_value_t = 5)]
        tweets_per_item: usize,
        #[arg(long = "for-you")]
        for_you: bool,
        #[arg(long = "news-only")]
        news_only: bool,
        #[arg(long)]
        sports: bool,
        #[arg(long)]
        entertainment: bool,
        #[arg(long = "trending-only")]
        trending_only: bool,
        #[arg(long)]
        json: bool,
        #[arg(long = "json-full")]
        json_full: bool,
    },
    #[command(name = "query-ids")]
    QueryIds {
        #[arg(long)]
        json: bool,
        #[arg(long)]
        fresh: bool,
    },
    Home {
        #[arg(short = 'n', long, default_value_t = 20)]
        count: usize,
        #[arg(long)]
        following: bool,
        #[arg(long)]
        json: bool,
        #[arg(long = "json-full")]
        json_full: bool,
    },
    Read {
        tweet_id_or_url: String,
        #[arg(long)]
        json: bool,
        #[arg(long = "json-full")]
        json_full: bool,
    },
    Replies {
        tweet_id_or_url: String,
        #[arg(long)]
        all: bool,
        #[arg(long = "max-pages")]
        max_pages: Option<usize>,
        #[arg(long, default_value_t = 1000)]
        delay: u64,
        #[arg(long)]
        cursor: Option<String>,
        #[arg(long)]
        json: bool,
        #[arg(long = "json-full")]
        json_full: bool,
    },
    Thread {
        tweet_id_or_url: String,
        #[arg(long)]
        all: bool,
        #[arg(long = "max-pages")]
        max_pages: Option<usize>,
        #[arg(long, default_value_t = 1000)]
        delay: u64,
        #[arg(long)]
        cursor: Option<String>,
        #[arg(long)]
        json: bool,
        #[arg(long = "json-full")]
        json_full: bool,
    },
    Search {
        query: String,
        #[arg(short = 'n', long, default_value_t = 10)]
        count: usize,
        #[arg(long)]
        all: bool,
        #[arg(long = "max-pages")]
        max_pages: Option<usize>,
        #[arg(long)]
        cursor: Option<String>,
        #[arg(long)]
        json: bool,
        #[arg(long = "json-full")]
        json_full: bool,
    },
    Mentions {
        #[arg(short = 'u', long)]
        user: Option<String>,
        #[arg(short = 'n', long, default_value_t = 10)]
        count: usize,
        #[arg(long)]
        json: bool,
        #[arg(long = "json-full")]
        json_full: bool,
    },
    #[command(name = "user-tweets")]
    UserTweets {
        handle: String,
        #[arg(short = 'n', long, default_value_t = 20)]
        count: usize,
        #[arg(long = "max-pages")]
        max_pages: Option<usize>,
        #[arg(long, default_value_t = 1000)]
        delay: u64,
        #[arg(long)]
        cursor: Option<String>,
        #[arg(long)]
        json: bool,
        #[arg(long = "json-full")]
        json_full: bool,
    },
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ConfigFile {
    chrome_profile: Option<String>,
    chrome_profile_dir: Option<String>,
    firefox_profile: Option<String>,
    cookie_source: Option<CookieSourceConfig>,
    cookie_timeout_ms: Option<u64>,
    timeout_ms: Option<u64>,
    quote_depth: Option<usize>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(untagged)]
enum CookieSourceConfig {
    One(CookieSourceArg),
    Many(Vec<CookieSourceArg>),
}

impl CookieSourceConfig {
    fn into_vec(self) -> Vec<CookieSourceArg> {
        match self {
            Self::One(value) => vec![value],
            Self::Many(values) => values,
        }
    }
}

impl ConfigFile {
    fn merge(self, other: Self) -> Self {
        Self {
            chrome_profile: other.chrome_profile.or(self.chrome_profile),
            chrome_profile_dir: other.chrome_profile_dir.or(self.chrome_profile_dir),
            firefox_profile: other.firefox_profile.or(self.firefox_profile),
            cookie_source: other.cookie_source.or(self.cookie_source),
            cookie_timeout_ms: other.cookie_timeout_ms.or(self.cookie_timeout_ms),
            timeout_ms: other.timeout_ms.or(self.timeout_ms),
            quote_depth: other.quote_depth.or(self.quote_depth),
        }
    }
}

struct CliContext {
    config: ConfigFile,
}

impl CliContext {
    fn load() -> Self {
        Self {
            config: load_config(),
        }
    }

    fn resolve_timeout(&self, global: &GlobalOptions) -> Option<Duration> {
        duration_from_first([
            global.timeout,
            self.config.timeout_ms,
            env_u64("BIRD_TIMEOUT_MS"),
        ])
    }

    fn resolve_cookie_timeout(&self, global: &GlobalOptions) -> Option<Duration> {
        duration_from_first([
            global.cookie_timeout,
            self.config.cookie_timeout_ms,
            env_u64("BIRD_COOKIE_TIMEOUT_MS"),
        ])
    }

    fn resolve_quote_depth(&self, global: &GlobalOptions) -> Option<usize> {
        first_some([
            global.quote_depth,
            self.config.quote_depth,
            env_u64("BIRD_QUOTE_DEPTH").map(|value| value as usize),
        ])
    }

    fn resolve_chrome_profile(&self, global: &GlobalOptions) -> Option<String> {
        global
            .chrome_profile_dir
            .clone()
            .or_else(|| global.chrome_profile.clone())
            .or_else(|| self.config.chrome_profile_dir.clone())
            .or_else(|| self.config.chrome_profile.clone())
    }

    fn resolve_firefox_profile(&self, global: &GlobalOptions) -> Option<String> {
        global
            .firefox_profile
            .clone()
            .or_else(|| self.config.firefox_profile.clone())
    }

    fn resolve_cookie_sources(&self, global: &GlobalOptions) -> Vec<bird_core::CookieSource> {
        if !global.cookie_source.is_empty() {
            return global
                .cookie_source
                .iter()
                .copied()
                .map(Into::into)
                .collect();
        }

        if let Some(cookie_source) = self.config.cookie_source.clone() {
            return cookie_source.into_vec().into_iter().map(Into::into).collect();
        }

        vec![
            bird_core::CookieSource::Safari,
            bird_core::CookieSource::Chrome,
            bird_core::CookieSource::Firefox,
        ]
    }

    fn resolve_credentials(&self, global: &GlobalOptions) -> anyhow::Result<bird_core::ResolvedCredentials> {
        let transport = transport_from_env();
        resolve_credentials(
            ResolveCredentialsOptions {
                auth_token: global.auth_token.clone(),
                ct0: global.ct0.clone(),
                cookie_source: self.resolve_cookie_sources(global),
                chrome_profile: self.resolve_chrome_profile(global),
                firefox_profile: self.resolve_firefox_profile(global),
                cookie_timeout: self.resolve_cookie_timeout(global),
            },
            &transport,
        )
    }

    fn build_client(&self, global: &GlobalOptions, cookies: TwitterCookies) -> anyhow::Result<TwitterClient> {
        TwitterClient::new(TwitterClientOptions {
            cookies,
            user_agent: None,
            timeout: self.resolve_timeout(global),
            quote_depth: self.resolve_quote_depth(global),
        })
    }
}

fn main() {
    if let Err(error) = run() {
        eprintln!("[err] {}", error.message);
        process::exit(error.code);
    }
}

fn run() -> CliResult<()> {
    let cli = Cli::parse_from(normalize_args(env::args_os()));
    let context = CliContext::load();

    match cli.command {
        Commands::Check => run_check(&context, &cli.global),
        Commands::Transport { json } => run_transport(json),
        Commands::Whoami => run_whoami(&context, &cli.global),
        Commands::Tweet { text } => run_tweet(&context, &cli.global, &text),
        Commands::Reply {
            tweet_id_or_url,
            text,
        } => run_reply(&context, &cli.global, &tweet_id_or_url, &text),
        Commands::Unbookmark { tweet_id_or_url } => {
            run_unbookmark(&context, &cli.global, &tweet_id_or_url)
        }
        Commands::Follow { username_or_id } => {
            run_follow_action(&context, &cli.global, &username_or_id, true)
        }
        Commands::Unfollow { username_or_id } => {
            run_follow_action(&context, &cli.global, &username_or_id, false)
        }
        Commands::Likes {
            count,
            all,
            max_pages,
            cursor,
            json,
            json_full,
        } => run_likes(
            &context,
            &cli.global,
            count,
            SearchArgs {
                all,
                max_pages,
                cursor,
            },
            json,
            json_full,
        ),
        Commands::Following {
            user_id,
            count,
            cursor,
            all,
            max_pages,
            json,
        } => run_user_list(
            &context,
            &cli.global,
            true,
            user_id.as_deref(),
            count,
            SearchArgs {
                all,
                max_pages,
                cursor,
            },
            json,
        ),
        Commands::Followers {
            user_id,
            count,
            cursor,
            all,
            max_pages,
            json,
        } => run_user_list(
            &context,
            &cli.global,
            false,
            user_id.as_deref(),
            count,
            SearchArgs {
                all,
                max_pages,
                cursor,
            },
            json,
        ),
        Commands::About { username, json } => run_about(&context, &cli.global, &username, json),
        Commands::Bookmarks {
            count,
            folder_id,
            all,
            max_pages,
            cursor,
            expand_root_only,
            author_chain,
            author_only,
            full_chain_only,
            include_ancestor_branches,
            include_parent,
            thread_meta,
            sort_chronological,
            json,
            json_full,
        } => run_bookmarks(
            &context,
            &cli.global,
            count,
            folder_id.as_deref(),
            SearchArgs {
                all,
                max_pages,
                cursor,
            },
            BookmarkArgs {
                expand_root_only,
                author_chain,
                author_only,
                full_chain_only,
                include_ancestor_branches,
                include_parent,
                thread_meta,
                sort_chronological,
            },
            json,
            json_full,
        ),
        Commands::Lists { count, json } => run_lists(&context, &cli.global, count, json),
        Commands::ListTimeline {
            list_id_or_url,
            count,
            all,
            max_pages,
            cursor,
            json,
            json_full,
        } => run_list_timeline(
            &context,
            &cli.global,
            &list_id_or_url,
            count,
            SearchArgs {
                all,
                max_pages,
                cursor,
            },
            json,
            json_full,
        ),
        Commands::News {
            count,
            ai_only,
            with_tweets,
            tweets_per_item,
            for_you,
            news_only,
            sports,
            entertainment,
            trending_only,
            json,
            json_full,
        } => run_news(
            &context,
            &cli.global,
            count,
            NewsArgs {
                ai_only,
                with_tweets,
                tweets_per_item,
                for_you,
                news_only,
                sports,
                entertainment,
                trending_only,
            },
            json,
            json_full,
        ),
        Commands::QueryIds { json, fresh } => run_query_ids(json, fresh),
        Commands::Home {
            count,
            following,
            json,
            json_full,
        } => run_home(&context, &cli.global, count, following, json, json_full),
        Commands::Read {
            tweet_id_or_url,
            json,
            json_full,
        } => run_read(&context, &cli.global, &tweet_id_or_url, json, json_full),
        Commands::Replies {
            tweet_id_or_url,
            all,
            max_pages,
            delay,
            cursor,
            json,
            json_full,
        } => run_thread_like(
            &context,
            &cli.global,
            ThreadMode::Replies,
            &tweet_id_or_url,
            PaginationArgs {
                all,
                max_pages,
                delay,
                cursor,
            },
            json,
            json_full,
        ),
        Commands::Thread {
            tweet_id_or_url,
            all,
            max_pages,
            delay,
            cursor,
            json,
            json_full,
        } => run_thread_like(
            &context,
            &cli.global,
            ThreadMode::Thread,
            &tweet_id_or_url,
            PaginationArgs {
                all,
                max_pages,
                delay,
                cursor,
            },
            json,
            json_full,
        ),
        Commands::Search {
            query,
            count,
            all,
            max_pages,
            cursor,
            json,
            json_full,
        } => run_search(
            &context,
            &cli.global,
            &query,
            count,
            SearchArgs {
                all,
                max_pages,
                cursor,
            },
            json,
            json_full,
        ),
        Commands::Mentions {
            user,
            count,
            json,
            json_full,
        } => run_mentions(&context, &cli.global, user.as_deref(), count, json, json_full),
        Commands::UserTweets {
            handle,
            count,
            max_pages,
            delay,
            cursor,
            json,
            json_full,
        } => run_user_tweets(
            &context,
            &cli.global,
            &handle,
            count,
            UserTweetsArgs {
                max_pages,
                delay,
                cursor,
            },
            json,
            json_full,
        ),
    }
}

fn run_transport(json: bool) -> CliResult<()> {
    let info = transport_from_env().info();
    if json {
        println!("{}", to_pretty_json(&info)?);
    } else {
        print_transport_info(&info);
    }

    if info.valid {
        return Ok(());
    }

    Err(CliError::runtime(
        info.error
            .clone()
            .unwrap_or_else(|| "invalid transport configuration".to_owned()),
    ))
}

fn print_transport_info(info: &TransportInfo) {
    println!("backend: {}", info.backend);
    println!("mode: {}", info.mode);
    println!("platform: {}", info.platform);
    if let Some(profile) = &info.profile {
        println!("profile: {profile}");
    }
    if let Some(profile_source) = info.profile_source {
        println!("profile_source: {profile_source}");
    }
    println!("scope: {}", info.scope);
    println!("valid: {}", info.valid);
    if let Some(error) = &info.error {
        println!("error: {error}");
    }
}

fn run_check(context: &CliContext, global: &GlobalOptions) -> CliResult<()> {
    let resolved = context.resolve_credentials(global)?;
    let cookies = resolved.cookies;

    println!("[info] Credential check");
    println!("{}", "-".repeat(40));
    if let Some(auth_token) = cookies.auth_token.as_deref() {
        println!("[ok] auth_token: {}...", preview_secret(auth_token));
    } else {
        println!("[err] auth_token: not found");
    }
    if let Some(ct0) = cookies.ct0.as_deref() {
        println!("[ok] ct0: {}...", preview_secret(ct0));
    } else {
        println!("[err] ct0: not found");
    }
    if let Some(source) = cookies.source.as_deref() {
        println!("source: {source}");
    }
    if !resolved.warnings.is_empty() {
        println!("\n[warn] Warnings:");
        for warning in resolved.warnings {
            println!("  - {warning}");
        }
    }
    if cookies.auth_token.is_some() && cookies.ct0.is_some() {
        println!("\n[ok] Ready to tweet!");
        Ok(())
    } else {
        println!("\n[err] Missing credentials. Options:");
        println!("  1. Login to x.com in Safari/Chrome/Firefox");
        println!("  2. Set AUTH_TOKEN and CT0 environment variables");
        println!("  3. Use --auth-token and --ct0 flags");
        Err(CliError::runtime("Missing required credentials"))
    }
}

fn run_whoami(context: &CliContext, global: &GlobalOptions) -> CliResult<()> {
    let resolved = context.resolve_credentials(global)?;
    print_warnings(&resolved.warnings);
    ensure_credentials(&resolved.cookies)?;
    let credential_source = resolved
        .cookies
        .source
        .clone()
        .unwrap_or_else(|| "env/auto-detected cookies".to_owned());
    let client = context.build_client(global, resolved.cookies)?;
    let user = client.get_current_user()?;
    print_current_user(&user, &credential_source);
    Ok(())
}

fn run_tweet(context: &CliContext, global: &GlobalOptions, text: &str) -> CliResult<()> {
    let media = load_media(global)?;
    let resolved = context.resolve_credentials(global)?;
    print_warnings(&resolved.warnings);
    ensure_credentials(&resolved.cookies)?;
    if let Some(source) = resolved.cookies.source.as_deref() {
        eprintln!("source: {source}");
    }
    let client = context.build_client(global, resolved.cookies)?;
    let media_ids = upload_media_if_any(&client, &media)?;
    let result = client.tweet(text, media_ids.as_deref());
    if result.success {
        let tweet_id = result
            .tweet_id
            .as_deref()
            .ok_or_else(|| CliError::runtime("Tweet created but no ID returned"))?;
        println!("[ok] Tweet posted successfully!");
        println!("url: https://x.com/i/status/{tweet_id}");
        return Ok(());
    }
    Err(CliError::runtime(
        result
            .error
            .unwrap_or_else(|| "Failed to post tweet".to_owned()),
    ))
}

fn run_reply(
    context: &CliContext,
    global: &GlobalOptions,
    tweet_id_or_url: &str,
    text: &str,
) -> CliResult<()> {
    let media = load_media(global)?;
    let tweet_id = extract_tweet_id(tweet_id_or_url);
    let resolved = context.resolve_credentials(global)?;
    print_warnings(&resolved.warnings);
    ensure_credentials(&resolved.cookies)?;
    if let Some(source) = resolved.cookies.source.as_deref() {
        eprintln!("source: {source}");
    }
    eprintln!("[info] Replying to tweet: {tweet_id}");
    let client = context.build_client(global, resolved.cookies)?;
    let media_ids = upload_media_if_any(&client, &media)?;
    let result = client.reply(text, &tweet_id, media_ids.as_deref());
    if result.success {
        let tweet_id = result
            .tweet_id
            .as_deref()
            .ok_or_else(|| CliError::runtime("Reply created but no ID returned"))?;
        println!("[ok] Reply posted successfully!");
        println!("url: https://x.com/i/status/{tweet_id}");
        return Ok(());
    }
    Err(CliError::runtime(
        result
            .error
            .unwrap_or_else(|| "Failed to post reply".to_owned()),
    ))
}

fn run_unbookmark(
    context: &CliContext,
    global: &GlobalOptions,
    tweet_id_or_urls: &[String],
) -> CliResult<()> {
    let resolved = context.resolve_credentials(global)?;
    print_warnings(&resolved.warnings);
    ensure_credentials(&resolved.cookies)?;
    let client = context.build_client(global, resolved.cookies)?;
    let mut failures = 0usize;

    for input in tweet_id_or_urls {
        let tweet_id = extract_tweet_id(input);
        let result = client.unbookmark(&tweet_id);
        if result.success {
            println!("[ok] Removed bookmark for {tweet_id}");
        } else {
            failures += 1;
            eprintln!(
                "[err] Failed to remove bookmark for {tweet_id}: {}",
                result.error.unwrap_or_else(|| "Unknown error".to_owned())
            );
        }
    }

    if failures == 0 {
        return Ok(());
    }

    Err(CliError::runtime(format!(
        "{failures} bookmark removal operation(s) failed"
    )))
}

fn run_follow_action(
    context: &CliContext,
    global: &GlobalOptions,
    username_or_id: &str,
    follow: bool,
) -> CliResult<()> {
    let resolved = context.resolve_credentials(global)?;
    print_warnings(&resolved.warnings);
    ensure_credentials(&resolved.cookies)?;
    let client = context.build_client(global, resolved.cookies)?;
    let target = resolve_user_id(&client, username_or_id)?;
    let display_name = target
        .username
        .as_deref()
        .map(|username| format!("@{username}"))
        .unwrap_or_else(|| target.user_id.clone());
    let result = if follow {
        client.follow(&target.user_id)
    } else {
        client.unfollow(&target.user_id)
    };

    if result.success {
        let final_name = result
            .username
            .as_deref()
            .or(target.username.as_deref())
            .map(|username| format!("@{username}"))
            .unwrap_or(display_name);
        if follow {
            println!("[ok] Now following {final_name}");
        } else {
            println!("[ok] Unfollowed {final_name}");
        }
        return Ok(());
    }

    let action = if follow { "follow" } else { "unfollow" };
    Err(CliError::runtime(format!(
        "Failed to {action} {display_name}: {}",
        result.error.unwrap_or_else(|| "Unknown error".to_owned())
    )))
}

fn run_likes(
    context: &CliContext,
    global: &GlobalOptions,
    count: usize,
    search: SearchArgs,
    json_output: bool,
    json_full: bool,
) -> CliResult<()> {
    validate_positive_opt(search.max_pages, "--max-pages")?;
    let use_pagination = search.all || search.cursor.is_some();
    if search.max_pages.is_some() && !use_pagination {
        return Err(CliError::usage("--max-pages requires --all or --cursor."));
    }
    if !use_pagination && count == 0 {
        return Err(CliError::usage("Invalid --count. Expected a positive integer."));
    }

    let resolved = context.resolve_credentials(global)?;
    print_warnings(&resolved.warnings);
    ensure_credentials(&resolved.cookies)?;
    let client = context.build_client(global, resolved.cookies)?;
    let limit = if use_pagination { usize::MAX / 4 } else { count };
    let page = client.get_likes(limit, json_full, search.cursor.clone(), search.max_pages)?;
    print_tweets_result(
        &page,
        json_output || json_full,
        use_pagination,
        "No liked tweets found.",
    );
    Ok(())
}

fn run_user_list(
    context: &CliContext,
    global: &GlobalOptions,
    following: bool,
    user_id: Option<&str>,
    count: usize,
    search: SearchArgs,
    json_output: bool,
) -> CliResult<()> {
    validate_positive_opt(search.max_pages, "--max-pages")?;
    if search.max_pages.is_some() && !search.all {
        return Err(CliError::usage("--max-pages requires --all."));
    }
    if count == 0 {
        return Err(CliError::usage("Invalid --count. Expected a positive integer."));
    }

    let resolved = context.resolve_credentials(global)?;
    print_warnings(&resolved.warnings);
    ensure_credentials(&resolved.cookies)?;
    let client = context.build_client(global, resolved.cookies)?;
    let target_user_id = match user_id {
        Some(value) => value.to_owned(),
        None => client.get_current_user()?.id,
    };
    let use_pagination = search.all || search.cursor.is_some();
    let effective_max_pages = if search.all {
        Some(search.max_pages.unwrap_or(usize::MAX))
    } else {
        None
    };
    let page = if following {
        client.get_following(
            &target_user_id,
            count,
            search.cursor.clone(),
            effective_max_pages,
        )?
    } else {
        client.get_followers(
            &target_user_id,
            count,
            search.cursor.clone(),
            effective_max_pages,
        )?
    };
    print_users_result(&page, json_output, use_pagination);
    Ok(())
}

fn run_about(
    context: &CliContext,
    global: &GlobalOptions,
    username: &str,
    json_output: bool,
) -> CliResult<()> {
    let handle = normalize_handle(username)
        .ok_or_else(|| CliError::usage(format!("Invalid username: {username}")))?;
    let resolved = context.resolve_credentials(global)?;
    print_warnings(&resolved.warnings);
    ensure_credentials(&resolved.cookies)?;
    let client = context.build_client(global, resolved.cookies)?;
    let profile = client.get_user_about_account(&handle)?;
    if json_output {
        println!("{}", to_pretty_json(&profile)?);
        return Ok(());
    }
    print_about_profile(&profile, &handle);
    Ok(())
}

#[derive(Debug, Clone, Copy)]
struct BookmarkArgs {
    expand_root_only: bool,
    author_chain: bool,
    author_only: bool,
    full_chain_only: bool,
    include_ancestor_branches: bool,
    include_parent: bool,
    thread_meta: bool,
    sort_chronological: bool,
}

fn run_bookmarks(
    context: &CliContext,
    global: &GlobalOptions,
    count: usize,
    folder_id: Option<&str>,
    search: SearchArgs,
    args: BookmarkArgs,
    json_output: bool,
    json_full: bool,
) -> CliResult<()> {
    validate_positive_opt(search.max_pages, "--max-pages")?;
    let use_pagination = search.all || search.cursor.is_some();
    if search.max_pages.is_some() && !use_pagination {
        return Err(CliError::usage("--max-pages requires --all or --cursor."));
    }
    if !use_pagination && count == 0 {
        return Err(CliError::usage("Invalid --count. Expected a positive integer."));
    }

    let parsed_folder_id = match folder_id {
        Some(value) => Some(extract_bookmark_folder_id(value).ok_or_else(|| {
            CliError::usage(
                "Invalid --folder-id. Expected numeric ID or https://x.com/i/bookmarks/<id>.",
            )
        })?),
        None => None,
    };

    if args.author_chain && (args.author_only || args.full_chain_only) {
        eprintln!(
            "[warn] --author-chain already limits to the connected self-reply chain; other chain filters are redundant."
        );
    }
    if args.include_ancestor_branches && !args.full_chain_only {
        eprintln!("[warn] --include-ancestor-branches only applies with --full-chain-only.");
    }

    let resolved = context.resolve_credentials(global)?;
    print_warnings(&resolved.warnings);
    ensure_credentials(&resolved.cookies)?;
    let client = context.build_client(global, resolved.cookies)?;
    let limit = if use_pagination { usize::MAX / 4 } else { count };
    let result = if let Some(folder_id) = parsed_folder_id.as_deref() {
        client.get_bookmark_folder_timeline(
            folder_id,
            limit,
            json_full,
            search.cursor.clone(),
            search.max_pages,
        )?
    } else {
        client.get_bookmarks(limit, json_full, search.cursor.clone(), search.max_pages)?
    };
    let empty_message = if parsed_folder_id.is_some() {
        "No bookmarks found in folder."
    } else {
        "No bookmarks found."
    };
    if result.tweets.is_empty() {
        print_tweets_result(&result, json_output || json_full, use_pagination, empty_message);
        return Ok(());
    }

    let should_attempt_expand = args.expand_root_only
        || args.author_chain
        || args.author_only
        || args.full_chain_only;
    let should_fetch_thread = should_attempt_expand || args.thread_meta;
    let mut expanded_results = Vec::new();
    let mut thread_cache: HashMap<String, Vec<TweetData>> = HashMap::new();

    for (index, bookmark) in result.tweets.iter().cloned().enumerate() {
        let is_root = bookmark.in_reply_to_status_id.is_none();
        let mut thread_tweets = None;
        if should_fetch_thread && (!args.expand_root_only || is_root || args.thread_meta) {
            if index > 0 {
                std::thread::sleep(Duration::from_secs(1));
            }
            thread_tweets = fetch_bookmark_thread(&client, &bookmark, json_full, &mut thread_cache);
        }

        let mut output_tweets = vec![bookmark.clone()];
        if should_attempt_expand {
            if args.expand_root_only && !is_root {
                output_tweets = vec![bookmark.clone()];
            } else if let Some(thread_tweets) = thread_tweets.as_ref() {
                if args.author_chain {
                    output_tweets = filter_author_chain(thread_tweets, &bookmark);
                } else {
                    output_tweets = if args.full_chain_only {
                        filter_full_chain(thread_tweets, &bookmark, args.include_ancestor_branches)
                    } else {
                        thread_tweets.clone()
                    };
                    if args.author_only {
                        output_tweets = filter_author_only(&output_tweets, &bookmark);
                    }
                }
            }
        }

        if args.include_parent {
            if let Some(parent_id) = bookmark.in_reply_to_status_id.as_deref() {
                let already_included = output_tweets.iter().any(|tweet| tweet.id == parent_id);
                if !already_included {
                    if let Some(parent) = thread_tweets
                        .as_ref()
                        .and_then(|tweets| tweets.iter().find(|tweet| tweet.id == parent_id))
                        .cloned()
                    {
                        expanded_results.push(parent);
                    } else if let Ok(parent) = client.get_tweet(parent_id, json_full) {
                        expanded_results.push(parent);
                    }
                }
            }
        }

        expanded_results.extend(output_tweets);
    }

    let mut final_results = if args.thread_meta {
        expanded_results
            .iter()
            .cloned()
            .map(|tweet| {
                let cache_key = tweet
                    .conversation_id
                    .clone()
                    .unwrap_or_else(|| tweet.id.clone());
                let conversation_tweets = thread_cache
                    .get(&cache_key)
                    .cloned()
                    .unwrap_or_else(|| vec![tweet.clone()]);
                add_thread_metadata(tweet, &conversation_tweets)
            })
            .collect::<Vec<_>>()
    } else {
        expanded_results
    };

    final_results = unique_tweets(final_results);
    if args.sort_chronological {
        final_results.sort_by_key(tweet_sort_key);
    }

    print_tweets_result(
        &bird_core::TweetsPage {
            tweets: final_results,
            next_cursor: result.next_cursor,
        },
        json_output || json_full,
        use_pagination,
        empty_message,
    );
    Ok(())
}

fn run_lists(
    context: &CliContext,
    global: &GlobalOptions,
    count: usize,
    json_output: bool,
) -> CliResult<()> {
    if count == 0 {
        return Err(CliError::usage("Invalid --count. Expected a positive integer."));
    }
    let resolved = context.resolve_credentials(global)?;
    print_warnings(&resolved.warnings);
    ensure_credentials(&resolved.cookies)?;
    let client = context.build_client(global, resolved.cookies)?;
    let lists = client.get_owned_lists(count)?;
    if json_output {
        println!("{}", to_pretty_json(&lists)?);
        return Ok(());
    }
    if lists.is_empty() {
        println!("You do not own any lists.");
        return Ok(());
    }
    print_lists(&lists);
    Ok(())
}

#[derive(Debug, Clone, Copy)]
struct NewsArgs {
    ai_only: bool,
    with_tweets: bool,
    tweets_per_item: usize,
    for_you: bool,
    news_only: bool,
    sports: bool,
    entertainment: bool,
    trending_only: bool,
}

fn run_list_timeline(
    context: &CliContext,
    global: &GlobalOptions,
    list_id_or_url: &str,
    count: usize,
    search: SearchArgs,
    json_output: bool,
    json_full: bool,
) -> CliResult<()> {
    validate_positive_opt(search.max_pages, "--max-pages")?;
    let list_id = extract_list_id(list_id_or_url).ok_or_else(|| {
        CliError::usage(
            "Invalid list ID or URL. Expected numeric ID or https://x.com/i/lists/<id>.",
        )
    })?;
    let use_pagination = search.all || search.cursor.is_some() || search.max_pages.is_some();
    if !use_pagination && count == 0 {
        return Err(CliError::usage("Invalid --count. Expected a positive integer."));
    }

    let resolved = context.resolve_credentials(global)?;
    print_warnings(&resolved.warnings);
    ensure_credentials(&resolved.cookies)?;
    let client = context.build_client(global, resolved.cookies)?;
    let limit = if use_pagination { usize::MAX / 4 } else { count };
    let page = client.get_list_timeline(
        &list_id,
        limit,
        json_full,
        search.cursor.clone(),
        search.max_pages,
    )?;
    print_tweets_result(
        &page,
        json_output || json_full,
        use_pagination,
        "No tweets found in this list.",
    );
    Ok(())
}

fn run_news(
    context: &CliContext,
    global: &GlobalOptions,
    count: usize,
    args: NewsArgs,
    json_output: bool,
    json_full: bool,
) -> CliResult<()> {
    if count == 0 {
        return Err(CliError::usage("--count must be a positive number"));
    }
    if args.tweets_per_item == 0 {
        return Err(CliError::usage("--tweets-per-item must be a positive number"));
    }

    let resolved = context.resolve_credentials(global)?;
    print_warnings(&resolved.warnings);
    ensure_credentials(&resolved.cookies)?;
    let client = context.build_client(global, resolved.cookies)?;
    let mut tabs = Vec::new();
    if args.for_you {
        tabs.push("forYou".to_owned());
    }
    if args.news_only {
        tabs.push("news".to_owned());
    }
    if args.sports {
        tabs.push("sports".to_owned());
    }
    if args.entertainment {
        tabs.push("entertainment".to_owned());
    }
    if args.trending_only {
        tabs.push("trending".to_owned());
    }
    let items = client.get_news(
        count,
        json_full,
        args.with_tweets,
        args.tweets_per_item,
        args.ai_only,
        (!tabs.is_empty()).then_some(tabs),
    )?;
    print_news_items(
        &items,
        json_output || json_full,
        args.with_tweets.then_some(args.tweets_per_item),
    );
    Ok(())
}

fn run_query_ids(json_output: bool, fresh: bool) -> CliResult<()> {
    let transport = transport_from_env();
    let store = RuntimeQueryIdStore::default();
    if fresh {
        eprintln!("[info] Refreshing GraphQL query IDs...");
        let _ = store.refresh(&transport, &target_query_id_operations())?;
        eprintln!("[info] Refreshing feature overrides...");
        let _ = refresh_features_cache()?;
    }
    let snapshot = store.snapshot();
    if json_output {
        println!("{}", to_pretty_json(&snapshot)?);
        return Ok(());
    }
    print_query_ids_snapshot(&snapshot);
    Ok(())
}

fn run_home(
    context: &CliContext,
    global: &GlobalOptions,
    count: usize,
    following: bool,
    json_output: bool,
    json_full: bool,
) -> CliResult<()> {
    if count == 0 {
        return Err(CliError::usage("Invalid --count. Expected a positive integer."));
    }
    let resolved = context.resolve_credentials(global)?;
    print_warnings(&resolved.warnings);
    ensure_credentials(&resolved.cookies)?;
    let client = context.build_client(global, resolved.cookies)?;
    let tweets = if following {
        client.get_home_latest_timeline(count, json_full)?
    } else {
        client.get_home_timeline(count, json_full)?
    };
    let empty_message = if following {
        "No tweets found in Following timeline."
    } else {
        "No tweets found in For You timeline."
    };
    print_tweets(&tweets, json_output || json_full, empty_message, true);
    Ok(())
}

fn run_read(
    context: &CliContext,
    global: &GlobalOptions,
    tweet_id_or_url: &str,
    json_output: bool,
    json_full: bool,
) -> CliResult<()> {
    let tweet_id = extract_tweet_id(tweet_id_or_url);
    let resolved = context.resolve_credentials(global)?;
    print_warnings(&resolved.warnings);
    ensure_credentials(&resolved.cookies)?;
    let client = context.build_client(global, resolved.cookies)?;
    let tweet = client.get_tweet(&tweet_id, json_full)?;
    if json_output || json_full {
        println!("{}", to_pretty_json(&tweet)?);
        return Ok(());
    }
    print_tweets(&[tweet.clone()], false, "Tweet not found.", false);
    println!("{}", format_stats_line(&tweet));
    Ok(())
}

#[derive(Debug, Clone)]
struct PaginationArgs {
    all: bool,
    max_pages: Option<usize>,
    delay: u64,
    cursor: Option<String>,
}

#[derive(Debug, Clone, Copy)]
enum ThreadMode {
    Replies,
    Thread,
}

fn run_thread_like(
    context: &CliContext,
    global: &GlobalOptions,
    mode: ThreadMode,
    tweet_id_or_url: &str,
    pagination: PaginationArgs,
    json_output: bool,
    json_full: bool,
) -> CliResult<()> {
    validate_positive_opt(pagination.max_pages, "--max-pages")?;
    let use_pagination =
        pagination.all || pagination.cursor.is_some() || pagination.max_pages.is_some();
    let page_delay = Duration::from_millis(pagination.delay);
    let tweet_id = extract_tweet_id(tweet_id_or_url);
    let resolved = context.resolve_credentials(global)?;
    print_warnings(&resolved.warnings);
    ensure_credentials(&resolved.cookies)?;
    let client = context.build_client(global, resolved.cookies)?;
    let page = match mode {
        ThreadMode::Replies => client.get_replies(
            &tweet_id,
            json_full,
            pagination.cursor.clone(),
            if use_pagination {
                pagination.max_pages
            } else {
                Some(1)
            },
            page_delay,
        )?,
        ThreadMode::Thread => client.get_thread(
            &tweet_id,
            json_full,
            pagination.cursor.clone(),
            if use_pagination {
                pagination.max_pages
            } else {
                Some(1)
            },
            page_delay,
        )?,
    };
    let empty_message = match mode {
        ThreadMode::Replies => "No replies found.",
        ThreadMode::Thread => "No thread tweets found.",
    };
    print_tweets_result(&page, json_output || json_full, use_pagination, empty_message);
    Ok(())
}

#[derive(Debug, Clone)]
struct SearchArgs {
    all: bool,
    max_pages: Option<usize>,
    cursor: Option<String>,
}

fn run_search(
    context: &CliContext,
    global: &GlobalOptions,
    query: &str,
    count: usize,
    search: SearchArgs,
    json_output: bool,
    json_full: bool,
) -> CliResult<()> {
    validate_positive_opt(search.max_pages, "--max-pages")?;
    let use_pagination = search.all || search.cursor.is_some();
    if search.max_pages.is_some() && !use_pagination {
        return Err(CliError::usage("--max-pages requires --all or --cursor."));
    }
    if !use_pagination && count == 0 {
        return Err(CliError::usage("Invalid --count. Expected a positive integer."));
    }
    let resolved = context.resolve_credentials(global)?;
    print_warnings(&resolved.warnings);
    ensure_credentials(&resolved.cookies)?;
    let client = context.build_client(global, resolved.cookies)?;
    let page = if use_pagination {
        client.search(
            query,
            usize::MAX / 4,
            json_full,
            search.cursor.clone(),
            search.max_pages,
        )?
    } else {
        client.search(query, count, json_full, None, None)?
    };
    print_tweets_result(&page, json_output || json_full, use_pagination, "No tweets found.");
    Ok(())
}

fn run_mentions(
    context: &CliContext,
    global: &GlobalOptions,
    user: Option<&str>,
    count: usize,
    json_output: bool,
    json_full: bool,
) -> CliResult<()> {
    if count == 0 {
        return Err(CliError::usage("Invalid --count. Expected a positive integer."));
    }
    let resolved = context.resolve_credentials(global)?;
    print_warnings(&resolved.warnings);
    ensure_credentials(&resolved.cookies)?;
    let client = context.build_client(global, resolved.cookies.clone())?;
    let query = if let Some(user) = user {
        let handle = normalize_handle(user).ok_or_else(|| {
            CliError::usage(
                "Invalid --user handle. Expected something like @erlinhoxha (letters, digits, underscore; max 15).",
            )
        })?;
        format!("@{handle}")
    } else {
        let current = client.get_current_user()?;
        format!("@{}", current.username)
    };
    let page = client.search(&query, count, json_full, None, None)?;
    print_tweets(
        &page.tweets,
        json_output || json_full,
        "No mentions found.",
        true,
    );
    Ok(())
}

#[derive(Debug, Clone)]
struct UserTweetsArgs {
    max_pages: Option<usize>,
    delay: u64,
    cursor: Option<String>,
}

fn run_user_tweets(
    context: &CliContext,
    global: &GlobalOptions,
    handle: &str,
    count: usize,
    args: UserTweetsArgs,
    json_output: bool,
    json_full: bool,
) -> CliResult<()> {
    validate_positive_opt(args.max_pages, "--max-pages")?;
    if count == 0 {
        return Err(CliError::usage("Invalid --count. Expected a positive integer."));
    }
    let page_size = 20usize;
    let hard_max_pages = 10usize;
    let hard_max_tweets = page_size * hard_max_pages;
    if count > hard_max_tweets {
        return Err(CliError::usage(format!(
            "Invalid --count. Max {hard_max_tweets} tweets per run (safety cap: {hard_max_pages} pages). Use --cursor to continue."
        )));
    }
    if args.max_pages.is_some_and(|value| value > hard_max_pages) {
        return Err(CliError::usage(format!(
            "Invalid --max-pages. Expected a positive integer (max: {hard_max_pages})."
        )));
    }
    let username = normalize_handle(handle)
        .ok_or_else(|| CliError::usage(format!("Invalid handle: {handle}")))?;
    let resolved = context.resolve_credentials(global)?;
    print_warnings(&resolved.warnings);
    ensure_credentials(&resolved.cookies)?;
    let client = context.build_client(global, resolved.cookies)?;
    let (user_id, _resolved_username, _name) = client.get_user_id_by_username(&username)?;
    let page = client.get_user_tweets(
        &user_id,
        count,
        json_full,
        args.cursor.clone(),
        args.max_pages,
        Duration::from_millis(args.delay),
    )?;
    let wants_pagination_output =
        args.cursor.is_some() || args.max_pages.is_some() || count > page_size;
    print_tweets_result(
        &page,
        json_output || json_full,
        wants_pagination_output,
        &format!("No tweets found for @{username}."),
    );
    Ok(())
}

fn print_current_user(user: &CurrentUser, credential_source: &str) {
    println!("user: @{} ({})", user.username, user.name);
    println!("user_id: {}", user.id);
    println!("engine: graphql");
    println!("credentials: {credential_source}");
}

fn print_query_ids_snapshot(snapshot: &QueryIdSnapshot) {
    if !snapshot.cached {
        println!("[warn] No cached query IDs yet.");
        println!("[info] Run: bird query-ids --fresh");
        println!("features_path: {}", snapshot.features_path.display());
        return;
    }
    println!("[ok] GraphQL query IDs cached");
    println!("path: {}", snapshot.cache_path.display());
    if let Some(fetched_at) = snapshot.fetched_at.as_deref() {
        println!("fetched_at: {fetched_at}");
    }
    if let Some(is_fresh) = snapshot.is_fresh {
        println!("fresh: {}", if is_fresh { "yes" } else { "no" });
    }
    println!("ops: {}", snapshot.ids.len());
    println!("features_path: {}", snapshot.features_path.display());
    println!("features: {}", count_feature_overrides(&snapshot.features));
}

fn print_about_profile(profile: &AboutProfile, handle: &str) {
    println!("[info] Account information for @{handle}:");
    if let Some(value) = profile.account_based_in.as_deref() {
        println!("  Account based in: {value}");
    }
    if let Some(value) = profile.created_country_accurate {
        println!(
            "  Creation country accurate: {}",
            if value { "Yes" } else { "No" }
        );
    }
    if let Some(value) = profile.location_accurate {
        println!("  Location accurate: {}", if value { "Yes" } else { "No" });
    }
    if let Some(value) = profile.source.as_deref() {
        println!("source: {value}");
    }
    if let Some(value) = profile.learn_more_url.as_deref() {
        println!("  Learn more: {value}");
    }
}

fn print_users_result(page: &UsersPage, json_output: bool, use_pagination: bool) {
    if json_output {
        if use_pagination {
            println!(
                "{}",
                serde_json::to_string_pretty(&json!({
                    "users": page.users,
                    "nextCursor": page.next_cursor
                }))
                .unwrap_or_else(|_| "{}".to_owned())
            );
        } else {
            println!(
                "{}",
                serde_json::to_string_pretty(&page.users).unwrap_or_else(|_| "[]".to_owned())
            );
        }
        return;
    }

    if page.users.is_empty() {
        println!("No users found.");
        return;
    }

    print_users(&page.users);
    if let Some(cursor) = page.next_cursor.as_deref() {
        eprintln!("[info] Next cursor: {cursor}");
    }
}

fn print_users(users: &[TwitterUser]) {
    for user in users {
        println!("@{} ({})", user.username, user.name);
        if let Some(description) = user.description.as_deref() {
            println!("  {}", truncate_text(description, 100));
        }
        if let Some(followers) = user.followers_count {
            println!("  [info] {} followers", followers.to_string());
        }
        println!("{}", "─".repeat(50));
    }
}

fn print_lists(lists: &[TwitterList]) {
    for list in lists {
        let visibility = if list.is_private == Some(true) {
            "[private]"
        } else {
            "[public]"
        };
        println!("{} {}", list.name, visibility);
        if let Some(description) = list.description.as_deref() {
            println!("  {}", truncate_text(description, 100));
        }
        println!(
            "  [info] {} members",
            list.member_count.unwrap_or(0)
        );
        if let Some(owner) = list.owner.as_ref() {
            println!("  Owner: @{}", owner.username);
        }
        println!("  https://x.com/i/lists/{}", list.id);
        println!("{}", "─".repeat(50));
    }
}

fn print_news_items(items: &[NewsItem], json_output: bool, tweet_limit: Option<usize>) {
    if json_output {
        println!(
            "{}",
            serde_json::to_string_pretty(items).unwrap_or_else(|_| "[]".to_owned())
        );
        return;
    }
    if items.is_empty() {
        println!("No news items found.");
        return;
    }
    for item in items {
        let category = item
            .category
            .as_deref()
            .map(|value| format!("[{value}]"))
            .unwrap_or_default();
        println!("\n{} {}", category, item.headline);
        if let Some(description) = item.description.as_deref() {
            println!("  {description}");
        }
        let mut meta = Vec::new();
        if let Some(time_ago) = item.time_ago.as_deref() {
            meta.push(time_ago.to_owned());
        }
        if let Some(post_count) = item.post_count {
            meta.push(format!("{} posts", format_post_count(post_count)));
        }
        if !meta.is_empty() {
            println!("  {}", meta.join(" | "));
        }
        if let Some(url) = item.url.as_deref() {
            println!("  url: {url}");
        }
        if let Some(tweets) = item.tweets.as_ref() {
            if !tweets.is_empty() {
                println!("  Related tweets:");
                let limit = tweet_limit.unwrap_or(tweets.len());
                for tweet in tweets.iter().take(limit) {
                    println!("    @{}: {}", tweet.author.username, truncate_text(&tweet.text, 100));
                }
            }
        }
        println!("{}", "─".repeat(50));
    }
}

fn format_post_count(count: u64) -> String {
    if count >= 1_000_000 {
        return format!("{:.1}M", count as f64 / 1_000_000f64);
    }
    if count >= 1_000 {
        return format!("{:.1}K", count as f64 / 1_000f64);
    }
    count.to_string()
}

fn fetch_bookmark_thread(
    client: &TwitterClient,
    bookmark: &TweetData,
    include_raw: bool,
    cache: &mut HashMap<String, Vec<TweetData>>,
) -> Option<Vec<TweetData>> {
    let cached_key = bookmark
        .conversation_id
        .clone()
        .unwrap_or_else(|| bookmark.id.clone());
    if let Some(cached) = cache.get(&cached_key) {
        return Some(cached.clone());
    }
    let result = client
        .get_thread(&bookmark.id, include_raw, None, Some(1), Duration::ZERO)
        .ok()?;
    if result.tweets.is_empty() {
        return None;
    }
    let root_key = result
        .tweets
        .first()
        .and_then(|tweet| tweet.conversation_id.clone())
        .unwrap_or(cached_key);
    cache.insert(root_key.clone(), result.tweets.clone());
    Some(result.tweets)
}

fn filter_author_chain(tweets: &[TweetData], bookmarked_tweet: &TweetData) -> Vec<TweetData> {
    let author = bookmarked_tweet.author.username.as_str();
    let by_id = tweets
        .iter()
        .cloned()
        .map(|tweet| (tweet.id.clone(), tweet))
        .collect::<HashMap<_, _>>();
    let mut chain_ids = BTreeSet::new();
    let mut current = Some(bookmarked_tweet);
    while let Some(tweet) = current {
        if tweet.author.username != author {
            break;
        }
        chain_ids.insert(tweet.id.clone());
        let Some(parent_id) = tweet.in_reply_to_status_id.as_deref() else {
            break;
        };
        let Some(parent) = by_id.get(parent_id) else {
            break;
        };
        if parent.author.username != author {
            break;
        }
        current = Some(parent);
    }

    let mut changed = true;
    while changed {
        changed = false;
        for tweet in tweets {
            if tweet.author.username != author || chain_ids.contains(&tweet.id) {
                continue;
            }
            if tweet
                .in_reply_to_status_id
                .as_deref()
                .is_some_and(|parent_id| chain_ids.contains(parent_id))
            {
                chain_ids.insert(tweet.id.clone());
                changed = true;
            }
        }
    }

    let mut filtered = tweets
        .iter()
        .filter(|tweet| chain_ids.contains(&tweet.id))
        .cloned()
        .collect::<Vec<_>>();
    filtered.sort_by_key(tweet_sort_key);
    filtered
}

fn filter_author_only(tweets: &[TweetData], bookmarked_tweet: &TweetData) -> Vec<TweetData> {
    let author = bookmarked_tweet.author.username.as_str();
    tweets
        .iter()
        .filter(|tweet| tweet.author.username == author)
        .cloned()
        .collect()
}

fn filter_full_chain(
    tweets: &[TweetData],
    bookmarked_tweet: &TweetData,
    include_ancestor_branches: bool,
) -> Vec<TweetData> {
    let by_id = tweets
        .iter()
        .cloned()
        .map(|tweet| (tweet.id.clone(), tweet))
        .collect::<HashMap<_, _>>();
    let mut replies_by_parent: HashMap<String, Vec<TweetData>> = HashMap::new();
    for tweet in tweets {
        let Some(parent_id) = tweet.in_reply_to_status_id.as_deref() else {
            continue;
        };
        replies_by_parent
            .entry(parent_id.to_owned())
            .or_default()
            .push(tweet.clone());
    }

    let mut chain_ids = BTreeSet::new();
    let mut ancestor_ids = Vec::new();
    chain_ids.insert(bookmarked_tweet.id.clone());
    let mut current = bookmarked_tweet.clone();
    while let Some(parent_id) = current.in_reply_to_status_id.as_deref() {
        let Some(parent) = by_id.get(parent_id) else {
            break;
        };
        if chain_ids.insert(parent.id.clone()) {
            ancestor_ids.push(parent.id.clone());
        }
        current = parent.clone();
    }

    add_descendants(&mut chain_ids, &replies_by_parent, &[bookmarked_tweet.id.clone()]);
    if include_ancestor_branches {
        for ancestor_id in ancestor_ids {
            add_descendants(&mut chain_ids, &replies_by_parent, &[ancestor_id]);
        }
    }

    let mut filtered = tweets
        .iter()
        .filter(|tweet| chain_ids.contains(&tweet.id))
        .cloned()
        .collect::<Vec<_>>();
    filtered.sort_by_key(tweet_sort_key);
    filtered
}

fn add_descendants(
    chain_ids: &mut BTreeSet<String>,
    replies_by_parent: &HashMap<String, Vec<TweetData>>,
    start_ids: &[String],
) {
    let mut queue = start_ids.to_vec();
    while let Some(current_id) = queue.pop() {
        chain_ids.insert(current_id.clone());
        if let Some(replies) = replies_by_parent.get(&current_id) {
            for reply in replies {
                if chain_ids.insert(reply.id.clone()) {
                    queue.push(reply.id.clone());
                }
            }
        }
    }
}

fn add_thread_metadata(mut tweet: TweetData, all_conversation_tweets: &[TweetData]) -> TweetData {
    let author = tweet.author.username.clone();
    let has_self_replies = all_conversation_tweets.iter().any(|candidate| {
        candidate.in_reply_to_status_id.as_deref() == Some(tweet.id.as_str())
            && candidate.author.username == author
    });
    let is_root = tweet.in_reply_to_status_id.is_none();
    let thread_position = if is_root && !has_self_replies {
        "standalone"
    } else if is_root {
        "root"
    } else if has_self_replies {
        "middle"
    } else {
        "end"
    };

    tweet.is_thread = Some(has_self_replies || !is_root);
    tweet.thread_position = Some(thread_position.to_owned());
    tweet.has_self_replies = Some(has_self_replies);
    tweet.thread_root_id = Some(
        tweet.conversation_id
            .clone()
            .unwrap_or_else(|| tweet.id.clone()),
    );
    tweet
}

fn unique_tweets(tweets: Vec<TweetData>) -> Vec<TweetData> {
    let mut seen = BTreeSet::new();
    let mut unique = Vec::new();
    for tweet in tweets {
        if seen.insert(tweet.id.clone()) {
            unique.push(tweet);
        }
    }
    unique
}

fn tweet_sort_key(tweet: &TweetData) -> (i32, u32, u32, u32, u32, u32, String) {
    let (year, month, day, hour, minute, second) =
        parse_created_at(tweet.created_at.as_deref()).unwrap_or((0, 0, 0, 0, 0, 0));
    (
        year,
        month,
        day,
        hour,
        minute,
        second,
        tweet.id.clone(),
    )
}

fn print_tweets_result(page: &bird_core::TweetsPage, json_output: bool, use_pagination: bool, empty_message: &str) {
    if json_output && use_pagination {
        println!(
            "{}",
            serde_json::to_string_pretty(&json!({
                "tweets": page.tweets,
                "nextCursor": page.next_cursor
            }))
            .unwrap_or_else(|_| "{}".to_owned())
        );
        return;
    }
    print_tweets(&page.tweets, json_output, empty_message, true);
}

fn print_tweets(tweets: &[TweetData], json_output: bool, empty_message: &str, show_separator: bool) {
    if json_output {
        println!(
            "{}",
            serde_json::to_string_pretty(tweets).unwrap_or_else(|_| "[]".to_owned())
        );
        return;
    }
    if tweets.is_empty() {
        println!("{empty_message}");
        return;
    }
    for tweet in tweets {
        println!("\n@{} ({}):", tweet.author.username, tweet.author.name);
        if let Some(article) = tweet.article.as_ref() {
            if tweet.text.starts_with(&article.title) {
                println!("Article: {}", tweet.text);
            } else {
                println!("Article: {}", article.title);
                if let Some(preview) = article.preview_text.as_deref() {
                    println!("   {preview}");
                }
            }
        } else {
            println!("{}", tweet.text);
        }
        if let Some(media) = tweet.media.as_ref() {
            for item in media {
                println!("{} {}", media_label(&item.media_type), item.url);
            }
        }
        if let Some(quoted) = tweet.quoted_tweet.as_deref() {
            println!("> QT @{}:", quoted.author.username);
            let quoted_text = quoted
                .article
                .as_ref()
                .map(|article| format!("Article: {}", article.title))
                .unwrap_or_else(|| quoted.text.clone());
            for line in truncate_text(&quoted_text, 280).lines().take(4) {
                println!("> {line}");
            }
            if let Some(media) = quoted.media.as_ref() {
                for item in media {
                    println!("> {} {}", media_label(&item.media_type), item.url);
                }
            }
            println!(
                "> https://x.com/{}/status/{}",
                quoted.author.username, quoted.id
            );
        }
        if let Some(created_at) = tweet.created_at.as_deref() {
            println!("date: {created_at}");
        }
        println!(
            "url: https://x.com/{}/status/{}",
            tweet.author.username, tweet.id
        );
        if show_separator {
            println!("{}", "─".repeat(50));
        }
    }
}

fn media_label(media_type: &str) -> &'static str {
    match media_type {
        "video" => "VIDEO:",
        "animated_gif" => "GIF:",
        _ => "PHOTO:",
    }
}

fn truncate_text(value: &str, max_len: usize) -> String {
    let mut truncated = value.chars().take(max_len + 1).collect::<String>();
    if truncated.chars().count() > max_len {
        truncated = truncated.chars().take(max_len).collect::<String>();
        truncated.push_str("...");
    }
    truncated
}

fn format_stats_line(tweet: &TweetData) -> String {
    format!(
        "likes: {}  retweets: {}  replies: {}",
        tweet.like_count.unwrap_or(0),
        tweet.retweet_count.unwrap_or(0),
        tweet.reply_count.unwrap_or(0)
    )
}

#[derive(Debug, Clone)]
struct MediaSpec {
    data: Vec<u8>,
    mime: String,
    alt: Option<String>,
}

#[derive(Debug, Clone)]
struct ResolvedUserTarget {
    user_id: String,
    username: Option<String>,
}

fn load_media(global: &GlobalOptions) -> CliResult<Vec<MediaSpec>> {
    if global.media.is_empty() {
        return Ok(Vec::new());
    }

    let mut specs = Vec::with_capacity(global.media.len());
    for (index, path) in global.media.iter().enumerate() {
        let mime = detect_mime(path).ok_or_else(|| {
            CliError::usage(format!(
                "Unsupported media type for {path}. Supported: jpg, jpeg, png, webp, gif, mp4, mov"
            ))
        })?;
        let data = fs::read(path)
            .map_err(|error| CliError::runtime(format!("Failed to read media file {path}: {error}")))?;
        specs.push(MediaSpec {
            data,
            mime: mime.to_owned(),
            alt: global.alt.get(index).cloned(),
        });
    }

    let video_count = specs
        .iter()
        .filter(|media| media.mime.starts_with("video/"))
        .count();
    if video_count > 1 {
        return Err(CliError::usage("Only one video can be attached"));
    }
    if video_count == 1 && specs.len() > 1 {
        return Err(CliError::usage("Video cannot be combined with other media"));
    }
    if specs.len() > 4 {
        return Err(CliError::usage("Maximum 4 media attachments"));
    }

    Ok(specs)
}

fn upload_media_if_any(client: &TwitterClient, media: &[MediaSpec]) -> CliResult<Option<Vec<String>>> {
    if media.is_empty() {
        return Ok(None);
    }
    let mut uploaded = Vec::with_capacity(media.len());
    for item in media {
        let result = client.upload_media(&item.data, &item.mime, item.alt.as_deref());
        if !result.success {
            return Err(CliError::runtime(format!(
                "Media upload failed: {}",
                result.error.unwrap_or_else(|| "Unknown error".to_owned())
            )));
        }
        let media_id = result
            .media_id
            .ok_or_else(|| CliError::runtime("Media upload did not return media_id"))?;
        uploaded.push(media_id);
    }
    Ok(Some(uploaded))
}

fn detect_mime(path: &str) -> Option<&'static str> {
    let path = path.to_ascii_lowercase();
    if path.ends_with(".jpg") || path.ends_with(".jpeg") {
        return Some("image/jpeg");
    }
    if path.ends_with(".png") {
        return Some("image/png");
    }
    if path.ends_with(".webp") {
        return Some("image/webp");
    }
    if path.ends_with(".gif") {
        return Some("image/gif");
    }
    if path.ends_with(".mp4") || path.ends_with(".m4v") {
        return Some("video/mp4");
    }
    if path.ends_with(".mov") {
        return Some("video/quicktime");
    }
    None
}

fn preview_secret(value: &str) -> String {
    value.chars().take(10).collect()
}

fn print_warnings(warnings: &[String]) {
    for warning in warnings {
        eprintln!("[warn] {warning}");
    }
}

fn ensure_credentials(cookies: &TwitterCookies) -> CliResult<()> {
    if cookies.auth_token.is_some() && cookies.ct0.is_some() {
        return Ok(());
    }
    Err(CliError::runtime("Missing required credentials"))
}

fn validate_positive_opt(value: Option<usize>, flag_name: &str) -> CliResult<()> {
    if value == Some(0) {
        return Err(CliError::usage(format!(
            "Invalid {flag_name}. Expected a positive integer."
        )));
    }
    Ok(())
}

fn transport_from_env() -> CurlTransport {
    CurlTransport::new(env::var("TWITTER_PROXY").ok().filter(|value| !value.trim().is_empty()))
}

fn first_some<T>(values: impl IntoIterator<Item = Option<T>>) -> Option<T> {
    values.into_iter().flatten().next()
}

fn duration_from_first(values: impl IntoIterator<Item = Option<u64>>) -> Option<Duration> {
    first_some(values).filter(|value| *value > 0).map(Duration::from_millis)
}

fn to_pretty_json<T: serde::Serialize>(value: &T) -> CliResult<String> {
    serde_json::to_string_pretty(value)
        .map_err(anyhow::Error::from)
        .map_err(CliError::from)
}

fn env_u64(key: &str) -> Option<u64> {
    env::var(key)
        .ok()
        .map(|value| value.trim().to_owned())
        .filter(|value| !value.is_empty())
        .and_then(|value| value.parse::<u64>().ok())
}

fn load_config() -> ConfigFile {
    let global = dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".config/bird/config.json5");
    let local = env::current_dir()
        .unwrap_or_else(|_| PathBuf::from("."))
        .join(".birdrc.json5");
    read_config_file(&global).merge(read_config_file(&local))
}

fn read_config_file(path: &Path) -> ConfigFile {
    let Ok(raw) = fs::read_to_string(path) else {
        return ConfigFile::default();
    };
    match json5::from_str::<ConfigFile>(&raw) {
        Ok(config) => config,
        Err(error) => {
            eprintln!(
                "[warn] Failed to parse config at {}: {error}",
                path.display()
            );
            ConfigFile::default()
        }
    }
}

fn count_feature_overrides(value: &serde_json::Value) -> usize {
    let mut count = 0usize;
    if let Some(global) = value.get("global").and_then(serde_json::Value::as_object) {
        count += global.len();
    }
    if let Some(sets) = value.get("sets").and_then(serde_json::Value::as_object) {
        for overrides in sets.values() {
            if let Some(map) = overrides.as_object() {
                count += map.len();
            }
        }
    }
    count
}

fn extract_tweet_id(input: &str) -> String {
    for needle in ["twitter.com/", "x.com/"] {
        if let Some(index) = input.find(needle) {
            let tail = &input[index + needle.len()..];
            if let Some(status_index) = tail.find("/status/") {
                let id = tail[status_index + "/status/".len()..]
                    .chars()
                    .take_while(|ch| ch.is_ascii_digit())
                    .collect::<String>();
                if !id.is_empty() {
                    return id;
                }
            }
            if let Some(status_index) = tail.find("/web/status/") {
                let id = tail[status_index + "/web/status/".len()..]
                    .chars()
                    .take_while(|ch| ch.is_ascii_digit())
                    .collect::<String>();
                if !id.is_empty() {
                    return id;
                }
            }
        }
    }
    input.to_owned()
}

fn extract_list_id(input: &str) -> Option<String> {
    let trimmed = input.trim();
    if trimmed.len() >= 5 && trimmed.chars().all(|ch| ch.is_ascii_digit()) {
        return Some(trimmed.to_owned());
    }

    for needle in ["twitter.com/i/lists/", "x.com/i/lists/"] {
        if let Some(index) = trimmed.find(needle) {
            let id = trimmed[index + needle.len()..]
                .chars()
                .take_while(|ch| ch.is_ascii_digit())
                .collect::<String>();
            if id.len() >= 5 {
                return Some(id);
            }
        }
    }

    None
}

fn extract_bookmark_folder_id(input: &str) -> Option<String> {
    let trimmed = input.trim();
    if trimmed.len() >= 5 && trimmed.chars().all(|ch| ch.is_ascii_digit()) {
        return Some(trimmed.to_owned());
    }

    for needle in ["twitter.com/i/bookmarks/", "x.com/i/bookmarks/"] {
        if let Some(index) = trimmed.find(needle) {
            let id = trimmed[index + needle.len()..]
                .chars()
                .take_while(|ch| ch.is_ascii_digit())
                .collect::<String>();
            if id.len() >= 5 {
                return Some(id);
            }
        }
    }

    None
}

fn parse_created_at(value: Option<&str>) -> Option<(i32, u32, u32, u32, u32, u32)> {
    let value = value?;
    let mut parts = value.split_whitespace();
    let _weekday = parts.next()?;
    let month = month_number(parts.next()?)?;
    let day = parts.next()?.parse::<u32>().ok()?;
    let time = parts.next()?;
    let _offset = parts.next()?;
    let year = parts.next()?.parse::<i32>().ok()?;
    let mut time_parts = time.split(':');
    let hour = time_parts.next()?.parse::<u32>().ok()?;
    let minute = time_parts.next()?.parse::<u32>().ok()?;
    let second = time_parts.next()?.parse::<u32>().ok()?;
    Some((year, month, day, hour, minute, second))
}

fn month_number(value: &str) -> Option<u32> {
    match value {
        "Jan" => Some(1),
        "Feb" => Some(2),
        "Mar" => Some(3),
        "Apr" => Some(4),
        "May" => Some(5),
        "Jun" => Some(6),
        "Jul" => Some(7),
        "Aug" => Some(8),
        "Sep" => Some(9),
        "Oct" => Some(10),
        "Nov" => Some(11),
        "Dec" => Some(12),
        _ => None,
    }
}

fn normalize_handle(input: &str) -> Option<String> {
    let trimmed = input.trim().trim_start_matches('@');
    if trimmed.is_empty() || trimmed.len() > 15 {
        return None;
    }
    if trimmed
        .chars()
        .all(|ch| ch.is_ascii_alphanumeric() || ch == '_')
    {
        Some(trimmed.to_owned())
    } else {
        None
    }
}

fn resolve_user_id(client: &TwitterClient, username_or_id: &str) -> CliResult<ResolvedUserTarget> {
    let raw = username_or_id.trim();
    if raw.is_empty() {
        return Err(CliError::usage(
            "Invalid username or ID. Expected a numeric user ID or a handle like @erlinhoxha.",
        ));
    }

    let is_numeric = raw.chars().all(|ch| ch.is_ascii_digit());

    if let Some(handle) = normalize_handle(raw) {
        match client.get_user_id_by_username(&handle) {
            Ok((user_id, username, _name)) => {
                return Ok(ResolvedUserTarget {
                    user_id,
                    username: Some(username),
                });
            }
            Err(error) if !is_numeric => {
                return Err(CliError::runtime(format!(
                    "Failed to find user @{handle}: {error:#}"
                )));
            }
            Err(_) => {}
        }
    }

    if is_numeric {
        return Ok(ResolvedUserTarget {
            user_id: raw.to_owned(),
            username: None,
        });
    }

    Err(CliError::usage(format!(
        "Invalid username: {username_or_id}"
    )))
}

fn normalize_args(args: impl IntoIterator<Item = OsString>) -> Vec<OsString> {
    let args = args.into_iter().collect::<Vec<_>>();
    let mut normalized = Vec::with_capacity(args.len() + 1);
    let mut iter = args.into_iter();
    if let Some(program) = iter.next() {
        normalized.push(program);
    }

    let remaining = iter.collect::<Vec<_>>();
    let first_positional = first_positional_index(&remaining);
    if let Some(index) = first_positional {
        if let Some(value) = remaining[index].to_str() {
            if !KNOWN_COMMANDS.contains(&value) && !value.starts_with('-') {
                normalized.extend_from_slice(&remaining[..index]);
                normalized.push(OsString::from("read"));
                normalized.extend_from_slice(&remaining[index..]);
                return normalized;
            }
        }
    }

    normalized.extend(remaining);
    normalized
}

fn first_positional_index(args: &[OsString]) -> Option<usize> {
    let mut index = 0usize;
    while index < args.len() {
        let Some(value) = args[index].to_str() else {
            index += 1;
            continue;
        };
        if value == "--" {
            return (index + 1 < args.len()).then_some(index + 1);
        }
        if consumes_next_global_option(value) {
            index += 2;
            continue;
        }
        if value.starts_with("--") {
            index += 1;
            continue;
        }
        if value.starts_with('-') && value.len() > 1 {
            index += 1;
            continue;
        }
        return Some(index);
    }
    None
}

fn consumes_next_global_option(value: &str) -> bool {
    matches!(
        value,
        "--auth-token"
            | "--ct0"
            | "--chrome-profile"
            | "--chrome-profile-dir"
            | "--firefox-profile"
            | "--cookie-timeout"
            | "--cookie-source"
            | "--media"
            | "--alt"
            | "--timeout"
            | "--quote-depth"
    )
}

#[allow(dead_code)]
fn rebuild_cookie_header(cookies: &TwitterCookies) -> Option<String> {
    match (&cookies.auth_token, &cookies.ct0) {
        (Some(auth_token), Some(ct0)) => Some(build_cookie_header_from_cookies(
            &[],
            Some(auth_token.clone()),
            Some(ct0.clone()),
        )),
        _ => None,
    }
}
