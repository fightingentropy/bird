use std::collections::BTreeSet;

use serde_json::Value;

use crate::types::{
    TweetArticle, TweetAuthor, TweetData, TweetMedia, TwitterList, TwitterListOwner, TwitterUser,
};

pub fn normalize_quote_depth(value: Option<usize>) -> usize {
    value.unwrap_or(1)
}

pub fn parse_tweets_from_instructions(
    instructions: Option<&[Value]>,
    quote_depth: usize,
    include_raw: bool,
) -> Vec<TweetData> {
    let mut tweets = Vec::new();
    let mut seen = BTreeSet::new();
    for instruction in instructions.unwrap_or(&[]) {
        let Some(entries) = instruction.get("entries").and_then(Value::as_array) else {
            continue;
        };
        for entry in entries {
            for result in collect_tweet_results_from_entry(entry) {
                if let Some(tweet) = map_tweet_result(result, quote_depth, include_raw) {
                    if seen.insert(tweet.id.clone()) {
                        tweets.push(tweet);
                    }
                }
            }
        }
    }
    tweets
}

pub fn extract_cursor_from_instructions(
    instructions: Option<&[Value]>,
    cursor_type: &str,
) -> Option<String> {
    for instruction in instructions.unwrap_or(&[]) {
        let Some(entries) = instruction.get("entries").and_then(Value::as_array) else {
            continue;
        };
        for entry in entries {
            let Some(content) = entry.get("content") else {
                continue;
            };
            let Some(found_cursor_type) = content.get("cursorType").and_then(Value::as_str) else {
                continue;
            };
            if found_cursor_type == cursor_type {
                if let Some(value) = content.get("value").and_then(Value::as_str) {
                    if !value.is_empty() {
                        return Some(value.to_owned());
                    }
                }
            }
        }
    }
    None
}

pub fn find_tweet_in_instructions(instructions: Option<&[Value]>, tweet_id: &str) -> Option<Value> {
    for instruction in instructions.unwrap_or(&[]) {
        let Some(entries) = instruction.get("entries").and_then(Value::as_array) else {
            continue;
        };
        for entry in entries {
            let maybe_result = vget(entry, &["content", "itemContent", "tweet_results", "result"])
                .cloned();
            if maybe_result
                .as_ref()
                .and_then(|result| result.get("rest_id").and_then(Value::as_str))
                == Some(tweet_id)
            {
                return maybe_result;
            }
        }
    }
    None
}

pub fn parse_users_from_instructions(instructions: Option<&[Value]>) -> Vec<TwitterUser> {
    let mut users = Vec::new();
    let mut seen = BTreeSet::new();

    for instruction in instructions.unwrap_or(&[]) {
        let Some(entries) = instruction.get("entries").and_then(Value::as_array) else {
            continue;
        };
        for entry in entries {
            let Some(content) = entry.get("content") else {
                continue;
            };
            let raw_user_result = vget(content, &["itemContent", "user_results", "result"]);
            let user_result = raw_user_result
                .and_then(|value| {
                    if value.get("__typename").and_then(Value::as_str)
                        == Some("UserWithVisibilityResults")
                    {
                        value.get("user")
                    } else {
                        Some(value)
                    }
                })
                .filter(|value| value.get("__typename").and_then(Value::as_str) == Some("User"));
            let Some(user_result) = user_result else {
                continue;
            };

            let legacy = user_result.get("legacy");
            let core = user_result.get("core");
            let user_id = user_result.get("rest_id").and_then(Value::as_str);
            let username = first_text(&[
                legacy.and_then(|value| value.get("screen_name")),
                core.and_then(|value| value.get("screen_name")),
            ]);
            let Some((user_id, username)) = user_id.zip(username) else {
                continue;
            };
            if !seen.insert(user_id.to_owned()) {
                continue;
            }

            users.push(TwitterUser {
                id: user_id.to_owned(),
                username: username.clone(),
                name: first_text(&[
                    legacy.and_then(|value| value.get("name")),
                    core.and_then(|value| value.get("name")),
                ])
                .unwrap_or(username),
                description: legacy
                    .and_then(|value| value.get("description"))
                    .and_then(Value::as_str)
                    .map(ToOwned::to_owned),
                followers_count: legacy
                    .and_then(|value| value.get("followers_count"))
                    .and_then(Value::as_u64),
                following_count: legacy
                    .and_then(|value| value.get("friends_count"))
                    .and_then(Value::as_u64),
                is_blue_verified: user_result
                    .get("is_blue_verified")
                    .and_then(Value::as_bool),
                profile_image_url: first_text(&[
                    legacy.and_then(|value| value.get("profile_image_url_https")),
                    vget(user_result, &["avatar", "image_url"]),
                ]),
                created_at: first_text(&[
                    legacy.and_then(|value| value.get("created_at")),
                    core.and_then(|value| value.get("created_at")),
                ]),
            });
        }
    }

    users
}

pub fn parse_lists_from_instructions(instructions: Option<&[Value]>) -> Vec<TwitterList> {
    let mut lists = Vec::new();
    let mut seen = BTreeSet::new();

    for instruction in instructions.unwrap_or(&[]) {
        let Some(entries) = instruction.get("entries").and_then(Value::as_array) else {
            continue;
        };
        for entry in entries {
            let list_result = vget(entry, &["content", "itemContent", "list"]);
            let Some(parsed) = list_result.and_then(parse_list_result) else {
                continue;
            };
            if seen.insert(parsed.id.clone()) {
                lists.push(parsed);
            }
        }
    }

    lists
}

pub fn map_tweet_result(result: &Value, quote_depth: usize, include_raw: bool) -> Option<TweetData> {
    let result = unwrap_tweet_result(result);
    let user_result = vget(
        result,
        &["core", "user_results", "result"],
    )?;
    let user_legacy = user_result.get("legacy");
    let user_core = user_result.get("core");
    let username = first_text(&[
        user_legacy.and_then(|value| value.get("screen_name")),
        user_core.and_then(|value| value.get("screen_name")),
    ])?;
    let name = first_text(&[
        user_legacy.and_then(|value| value.get("name")),
        user_core.and_then(|value| value.get("name")),
    ])
    .unwrap_or_else(|| username.clone());
    let id = result.get("rest_id").and_then(Value::as_str)?.to_owned();
    let text = extract_tweet_text(result)?;
    let quoted_tweet = if quote_depth > 0 {
        result
            .get("quoted_status_result")
            .and_then(|value| value.get("result"))
            .and_then(|quoted| map_tweet_result(quoted, quote_depth.saturating_sub(1), include_raw))
            .map(Box::new)
    } else {
        None
    };

    Some(TweetData {
        id,
        text,
        created_at: result
            .get("legacy")
            .and_then(|legacy| legacy.get("created_at"))
            .and_then(Value::as_str)
            .map(ToOwned::to_owned),
        reply_count: result
            .get("legacy")
            .and_then(|legacy| legacy.get("reply_count"))
            .and_then(Value::as_u64),
        retweet_count: result
            .get("legacy")
            .and_then(|legacy| legacy.get("retweet_count"))
            .and_then(Value::as_u64),
        like_count: result
            .get("legacy")
            .and_then(|legacy| legacy.get("favorite_count"))
            .and_then(Value::as_u64),
        conversation_id: result
            .get("legacy")
            .and_then(|legacy| legacy.get("conversation_id_str"))
            .and_then(Value::as_str)
            .map(ToOwned::to_owned),
        in_reply_to_status_id: result
            .get("legacy")
            .and_then(|legacy| legacy.get("in_reply_to_status_id_str"))
            .and_then(Value::as_str)
            .map(ToOwned::to_owned),
        author: TweetAuthor { username, name },
        author_id: user_result
            .get("rest_id")
            .and_then(Value::as_str)
            .map(ToOwned::to_owned),
        quoted_tweet,
        media: extract_media(result),
        article: extract_article_metadata(result),
        is_thread: None,
        thread_position: None,
        has_self_replies: None,
        thread_root_id: None,
        raw: include_raw.then(|| result.clone()),
    })
}

fn collect_tweet_results_from_entry(entry: &Value) -> Vec<&Value> {
    let mut results = Vec::new();
    let content = entry.get("content");
    for path in [
        &["itemContent", "tweet_results", "result"][..],
        &["item", "itemContent", "tweet_results", "result"][..],
    ] {
        if let Some(result) = content.and_then(|content| vget(content, path)) {
            if result.get("rest_id").is_some() {
                results.push(result);
            }
        }
    }
    if let Some(items) = content.and_then(|value| value.get("items")).and_then(Value::as_array) {
        for item in items {
            for path in [
                &["item", "itemContent", "tweet_results", "result"][..],
                &["itemContent", "tweet_results", "result"][..],
                &["content", "itemContent", "tweet_results", "result"][..],
            ] {
                if let Some(result) = vget(item, path) {
                    if result.get("rest_id").is_some() {
                        results.push(result);
                    }
                }
            }
        }
    }
    results
}

fn parse_list_result(value: &Value) -> Option<TwitterList> {
    let id = value.get("id_str").and_then(Value::as_str)?.to_owned();
    let name = value.get("name").and_then(Value::as_str)?.to_owned();
    let owner = vget(value, &["user_results", "result"]).and_then(parse_list_owner);

    Some(TwitterList {
        id,
        name,
        description: value
            .get("description")
            .and_then(Value::as_str)
            .map(ToOwned::to_owned),
        member_count: value.get("member_count").and_then(Value::as_u64),
        subscriber_count: value.get("subscriber_count").and_then(Value::as_u64),
        is_private: value
            .get("mode")
            .and_then(Value::as_str)
            .map(|mode| mode.eq_ignore_ascii_case("private")),
        created_at: value
            .get("created_at")
            .and_then(Value::as_str)
            .map(ToOwned::to_owned),
        owner,
    })
}

fn parse_list_owner(value: &Value) -> Option<TwitterListOwner> {
    let id = value.get("rest_id").and_then(Value::as_str)?.to_owned();
    let username = vget(value, &["legacy", "screen_name"])
        .and_then(Value::as_str)?
        .to_owned();
    let name = vget(value, &["legacy", "name"])
        .and_then(Value::as_str)
        .unwrap_or(&username)
        .to_owned();

    Some(TwitterListOwner { id, username, name })
}

fn unwrap_tweet_result<'a>(result: &'a Value) -> &'a Value {
    result.get("tweet").unwrap_or(result)
}

fn extract_tweet_text(result: &Value) -> Option<String> {
    extract_article_text(result)
        .or_else(|| extract_note_tweet_text(result))
        .or_else(|| {
            result
                .get("legacy")
                .and_then(|legacy| legacy.get("full_text"))
                .and_then(Value::as_str)
                .map(|text| text.trim().to_owned())
                .filter(|text| !text.is_empty())
        })
}

fn extract_note_tweet_text(result: &Value) -> Option<String> {
    first_text(&[
        vget(result, &["note_tweet", "note_tweet_results", "result", "text"]),
        vget(result, &["note_tweet", "note_tweet_results", "result", "richtext", "text"]),
        vget(result, &["note_tweet", "note_tweet_results", "result", "rich_text", "text"]),
    ])
}

fn extract_article_text(result: &Value) -> Option<String> {
    let article = result.get("article")?;
    let article_result = article
        .get("article_results")
        .and_then(|value| value.get("result"))
        .unwrap_or(article);
    let title = first_text(&[article_result.get("title"), article.get("title")]);
    let mut body = first_text(&[
        article_result.get("plain_text"),
        article.get("plain_text"),
        vget(article_result, &["body", "text"]),
        vget(article_result, &["content", "text"]),
        article_result.get("text"),
    ]);
    if body.as_ref().zip(title.as_ref()).map(|(body, title)| body == title).unwrap_or(false) {
        body = None;
    }
    if body.is_none() {
        let mut collected = Vec::new();
        collect_text_fields(article_result, &["text", "title"], &mut collected);
        collect_text_fields(article, &["text", "title"], &mut collected);
        let unique = unique_ordered(collected);
        let filtered = match title.as_ref() {
            Some(title) => unique
                .into_iter()
                .filter(|value| value != title)
                .collect::<Vec<_>>(),
            None => unique,
        };
        if !filtered.is_empty() {
            body = Some(filtered.join("\n\n"));
        }
    }
    match (title, body) {
        (Some(title), Some(body)) if !body.starts_with(&title) => Some(format!("{title}\n\n{body}")),
        (_, Some(body)) => Some(body),
        (Some(title), None) => Some(title),
        _ => None,
    }
}

fn extract_article_metadata(result: &Value) -> Option<TweetArticle> {
    let article = result.get("article")?;
    let article_result = article
        .get("article_results")
        .and_then(|value| value.get("result"))
        .unwrap_or(article);
    let title = first_text(&[article_result.get("title"), article.get("title")])?;
    let preview_text = first_text(&[
        article_result.get("preview_text"),
        article.get("preview_text"),
    ]);
    Some(TweetArticle { title, preview_text })
}

fn extract_media(result: &Value) -> Option<Vec<TweetMedia>> {
    let raw_media = result
        .get("legacy")
        .and_then(|legacy| {
            legacy
                .get("extended_entities")
                .and_then(|value| value.get("media"))
                .or_else(|| legacy.get("entities").and_then(|value| value.get("media")))
        })
        .and_then(Value::as_array)?;
    let media = raw_media
        .iter()
        .filter_map(|item| {
            let media_type = item.get("type").and_then(Value::as_str)?.to_owned();
            let url = item.get("media_url_https").and_then(Value::as_str)?.to_owned();
            let sizes = item.get("sizes");
            let width = sizes
                .and_then(|sizes| sizes.get("large").or_else(|| sizes.get("medium")))
                .and_then(|size| size.get("w"))
                .and_then(Value::as_u64);
            let height = sizes
                .and_then(|sizes| sizes.get("large").or_else(|| sizes.get("medium")))
                .and_then(|size| size.get("h"))
                .and_then(Value::as_u64);
            let preview_url = sizes
                .and_then(|sizes| sizes.get("small"))
                .map(|_| format!("{url}:small"));
            let (video_url, duration_ms) = if matches!(media_type.as_str(), "video" | "animated_gif")
            {
                let variants = item
                    .get("video_info")
                    .and_then(|value| value.get("variants"))
                    .and_then(Value::as_array)
                    .cloned()
                    .unwrap_or_default();
                let mut mp4_variants = variants
                    .into_iter()
                    .filter(|variant| {
                        variant
                            .get("content_type")
                            .and_then(Value::as_str)
                            == Some("video/mp4")
                    })
                    .collect::<Vec<_>>();
                mp4_variants.sort_by_key(|variant| variant.get("bitrate").and_then(Value::as_u64).unwrap_or_default());
                let video_url = mp4_variants
                    .last()
                    .and_then(|variant| variant.get("url"))
                    .and_then(Value::as_str)
                    .map(ToOwned::to_owned);
                let duration = item
                    .get("video_info")
                    .and_then(|value| value.get("duration_millis"))
                    .and_then(Value::as_u64);
                (video_url, duration)
            } else {
                (None, None)
            };
            Some(TweetMedia {
                media_type,
                url,
                width,
                height,
                preview_url,
                video_url,
                duration_ms,
            })
        })
        .collect::<Vec<_>>();
    if media.is_empty() {
        None
    } else {
        Some(media)
    }
}

fn first_text(values: &[Option<&Value>]) -> Option<String> {
    values
        .iter()
        .flatten()
        .filter_map(|value| value.as_str())
        .map(str::trim)
        .find(|value| !value.is_empty())
        .map(ToOwned::to_owned)
}

fn collect_text_fields(value: &Value, keys: &[&str], output: &mut Vec<String>) {
    match value {
        Value::String(_) => {}
        Value::Array(values) => {
            for value in values {
                collect_text_fields(value, keys, output);
            }
        }
        Value::Object(map) => {
            for (key, nested) in map {
                if keys.contains(&key.as_str()) {
                    if let Some(text) = nested.as_str().map(str::trim).filter(|text| !text.is_empty()) {
                        output.push(text.to_owned());
                    }
                }
                collect_text_fields(nested, keys, output);
            }
        }
        _ => {}
    }
}

fn unique_ordered(values: Vec<String>) -> Vec<String> {
    let mut seen = BTreeSet::new();
    let mut unique = Vec::new();
    for value in values {
        if seen.insert(value.clone()) {
            unique.push(value);
        }
    }
    unique
}

fn vget<'a>(value: &'a Value, path: &[&str]) -> Option<&'a Value> {
    let mut current = value;
    for segment in path {
        current = current.get(*segment)?;
    }
    Some(current)
}
