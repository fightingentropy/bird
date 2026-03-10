use std::collections::BTreeMap;
use std::fs;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
struct FeatureOverrides {
    #[serde(default)]
    global: BTreeMap<String, bool>,
    #[serde(default)]
    sets: BTreeMap<String, BTreeMap<String, bool>>,
}

pub fn features_path() -> PathBuf {
    std::env::var("BIRD_FEATURES_CACHE")
        .ok()
        .or_else(|| std::env::var("BIRD_FEATURES_PATH").ok())
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            dirs::home_dir()
                .unwrap_or_else(|| PathBuf::from("."))
                .join(".config/bird/features.json")
        })
}

pub fn features_snapshot() -> serde_json::Value {
    to_json_value(&load_feature_overrides())
}

pub fn refresh_features_cache() -> anyhow::Result<(PathBuf, serde_json::Value)> {
    let path = features_path();
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let payload = to_json_value(&load_feature_overrides());
    fs::write(&path, format!("{}\n", serde_json::to_string_pretty(&payload)?))?;
    Ok((path, payload))
}

pub fn build_home_timeline_features() -> Value {
    apply_feature_overrides("homeTimeline", build_timeline_features())
}

pub fn build_bookmarks_features() -> Value {
    let mut value = build_timeline_features();
    if let Some(map) = value.as_object_mut() {
        map.insert(
            "graphql_timeline_v2_bookmark_timeline".to_owned(),
            Value::Bool(true),
        );
    }
    apply_feature_overrides("bookmarks", value)
}

pub fn build_likes_features() -> Value {
    apply_feature_overrides("likes", build_timeline_features())
}

pub fn build_lists_features() -> Value {
    apply_feature_overrides(
        "lists",
        json!({
            "rweb_video_screen_enabled": true,
            "profile_label_improvements_pcf_label_in_post_enabled": true,
            "responsive_web_profile_redirect_enabled": true,
            "rweb_tipjar_consumption_enabled": true,
            "verified_phone_label_enabled": false,
            "creator_subscriptions_tweet_preview_api_enabled": true,
            "responsive_web_graphql_timeline_navigation_enabled": true,
            "responsive_web_graphql_exclude_directive_enabled": true,
            "responsive_web_graphql_skip_user_profile_image_extensions_enabled": false,
            "premium_content_api_read_enabled": false,
            "communities_web_enable_tweet_community_results_fetch": true,
            "c9s_tweet_anatomy_moderator_badge_enabled": true,
            "responsive_web_grok_analyze_button_fetch_trends_enabled": false,
            "responsive_web_grok_analyze_post_followups_enabled": false,
            "responsive_web_grok_annotations_enabled": false,
            "responsive_web_jetfuel_frame": true,
            "post_ctas_fetch_enabled": true,
            "responsive_web_grok_share_attachment_enabled": true,
            "articles_preview_enabled": true,
            "responsive_web_edit_tweet_api_enabled": true,
            "graphql_is_translatable_rweb_tweet_is_translatable_enabled": true,
            "view_counts_everywhere_api_enabled": true,
            "longform_notetweets_consumption_enabled": true,
            "responsive_web_twitter_article_tweet_consumption_enabled": true,
            "tweet_awards_web_tipping_enabled": false,
            "responsive_web_grok_show_grok_translated_post": false,
            "responsive_web_grok_analysis_button_from_backend": true,
            "creator_subscriptions_quote_tweet_preview_enabled": false,
            "freedom_of_speech_not_reach_fetch_enabled": true,
            "standardized_nudges_misinfo": true,
            "tweet_with_visibility_results_prefer_gql_limited_actions_policy_enabled": true,
            "longform_notetweets_rich_text_read_enabled": true,
            "longform_notetweets_inline_media_enabled": true,
            "responsive_web_grok_image_annotation_enabled": true,
            "responsive_web_grok_imagine_annotation_enabled": true,
            "responsive_web_grok_community_note_auto_translation_is_enabled": false,
            "responsive_web_enhance_cards_enabled": false,
            "blue_business_profile_image_shape_enabled": false,
            "responsive_web_text_conversations_enabled": false,
            "tweetypie_unmention_optimization_enabled": true,
            "vibe_api_enabled": false,
            "interactive_text_enabled": false
        }),
    )
}

pub fn build_following_features() -> Value {
    apply_feature_overrides(
        "following",
        json!({
            "rweb_video_screen_enabled": true,
            "profile_label_improvements_pcf_label_in_post_enabled": false,
            "responsive_web_profile_redirect_enabled": true,
            "rweb_tipjar_consumption_enabled": true,
            "verified_phone_label_enabled": false,
            "creator_subscriptions_tweet_preview_api_enabled": true,
            "responsive_web_graphql_timeline_navigation_enabled": true,
            "responsive_web_graphql_skip_user_profile_image_extensions_enabled": false,
            "premium_content_api_read_enabled": true,
            "communities_web_enable_tweet_community_results_fetch": true,
            "c9s_tweet_anatomy_moderator_badge_enabled": true,
            "responsive_web_grok_analyze_button_fetch_trends_enabled": false,
            "responsive_web_grok_analyze_post_followups_enabled": false,
            "responsive_web_grok_annotations_enabled": false,
            "responsive_web_jetfuel_frame": false,
            "post_ctas_fetch_enabled": true,
            "responsive_web_grok_share_attachment_enabled": false,
            "articles_preview_enabled": true,
            "responsive_web_edit_tweet_api_enabled": true,
            "graphql_is_translatable_rweb_tweet_is_translatable_enabled": true,
            "view_counts_everywhere_api_enabled": true,
            "longform_notetweets_consumption_enabled": true,
            "responsive_web_twitter_article_tweet_consumption_enabled": true,
            "tweet_awards_web_tipping_enabled": true,
            "responsive_web_grok_show_grok_translated_post": false,
            "responsive_web_grok_analysis_button_from_backend": false,
            "creator_subscriptions_quote_tweet_preview_enabled": false,
            "freedom_of_speech_not_reach_fetch_enabled": true,
            "standardized_nudges_misinfo": true,
            "tweet_with_visibility_results_prefer_gql_limited_actions_policy_enabled": true,
            "longform_notetweets_rich_text_read_enabled": true,
            "longform_notetweets_inline_media_enabled": true,
            "responsive_web_grok_image_annotation_enabled": false,
            "responsive_web_grok_imagine_annotation_enabled": false,
            "responsive_web_grok_community_note_auto_translation_is_enabled": false,
            "responsive_web_enhance_cards_enabled": false
        }),
    )
}

pub fn build_explore_features() -> Value {
    apply_feature_overrides(
        "explore",
        json!({
            "rweb_video_screen_enabled": true,
            "profile_label_improvements_pcf_label_in_post_enabled": true,
            "responsive_web_profile_redirect_enabled": true,
            "rweb_tipjar_consumption_enabled": true,
            "verified_phone_label_enabled": false,
            "creator_subscriptions_tweet_preview_api_enabled": true,
            "responsive_web_graphql_timeline_navigation_enabled": true,
            "responsive_web_graphql_exclude_directive_enabled": true,
            "responsive_web_graphql_skip_user_profile_image_extensions_enabled": false,
            "premium_content_api_read_enabled": false,
            "communities_web_enable_tweet_community_results_fetch": true,
            "c9s_tweet_anatomy_moderator_badge_enabled": true,
            "responsive_web_grok_analyze_button_fetch_trends_enabled": true,
            "responsive_web_grok_analyze_post_followups_enabled": true,
            "responsive_web_grok_annotations_enabled": true,
            "responsive_web_jetfuel_frame": true,
            "responsive_web_grok_share_attachment_enabled": true,
            "articles_preview_enabled": true,
            "responsive_web_edit_tweet_api_enabled": true,
            "graphql_is_translatable_rweb_tweet_is_translatable_enabled": true,
            "view_counts_everywhere_api_enabled": true,
            "longform_notetweets_consumption_enabled": true,
            "responsive_web_twitter_article_tweet_consumption_enabled": true,
            "tweet_awards_web_tipping_enabled": false,
            "responsive_web_grok_show_grok_translated_post": true,
            "responsive_web_grok_analysis_button_from_backend": true,
            "creator_subscriptions_quote_tweet_preview_enabled": false,
            "freedom_of_speech_not_reach_fetch_enabled": true,
            "standardized_nudges_misinfo": true,
            "tweet_with_visibility_results_prefer_gql_limited_actions_policy_enabled": true,
            "longform_notetweets_rich_text_read_enabled": true,
            "longform_notetweets_inline_media_enabled": true,
            "responsive_web_grok_image_annotation_enabled": true,
            "responsive_web_grok_imagine_annotation_enabled": true,
            "responsive_web_grok_community_note_auto_translation_is_enabled": true,
            "responsive_web_enhance_cards_enabled": false,
            "post_ctas_fetch_enabled": true,
            "rweb_video_timestamps_enabled": true
        }),
    )
}

pub fn build_search_features() -> Value {
    apply_feature_overrides(
        "search",
        json!({
            "rweb_video_screen_enabled": true,
            "profile_label_improvements_pcf_label_in_post_enabled": true,
            "responsive_web_profile_redirect_enabled": true,
            "rweb_tipjar_consumption_enabled": true,
            "verified_phone_label_enabled": false,
            "creator_subscriptions_tweet_preview_api_enabled": true,
            "responsive_web_graphql_timeline_navigation_enabled": true,
            "responsive_web_graphql_exclude_directive_enabled": true,
            "responsive_web_graphql_skip_user_profile_image_extensions_enabled": false,
            "premium_content_api_read_enabled": false,
            "communities_web_enable_tweet_community_results_fetch": true,
            "c9s_tweet_anatomy_moderator_badge_enabled": true,
            "responsive_web_grok_analyze_button_fetch_trends_enabled": false,
            "responsive_web_grok_analyze_post_followups_enabled": false,
            "responsive_web_grok_annotations_enabled": false,
            "responsive_web_jetfuel_frame": true,
            "post_ctas_fetch_enabled": true,
            "responsive_web_grok_share_attachment_enabled": true,
            "responsive_web_edit_tweet_api_enabled": true,
            "graphql_is_translatable_rweb_tweet_is_translatable_enabled": true,
            "view_counts_everywhere_api_enabled": true,
            "longform_notetweets_consumption_enabled": true,
            "responsive_web_twitter_article_tweet_consumption_enabled": true,
            "tweet_awards_web_tipping_enabled": false,
            "responsive_web_grok_show_grok_translated_post": false,
            "responsive_web_grok_analysis_button_from_backend": true,
            "creator_subscriptions_quote_tweet_preview_enabled": false,
            "freedom_of_speech_not_reach_fetch_enabled": true,
            "standardized_nudges_misinfo": true,
            "tweet_with_visibility_results_prefer_gql_limited_actions_policy_enabled": true,
            "rweb_video_timestamps_enabled": true,
            "longform_notetweets_rich_text_read_enabled": true,
            "longform_notetweets_inline_media_enabled": true,
            "responsive_web_grok_image_annotation_enabled": true,
            "responsive_web_grok_imagine_annotation_enabled": true,
            "responsive_web_grok_community_note_auto_translation_is_enabled": false,
            "articles_preview_enabled": true,
            "responsive_web_enhance_cards_enabled": false
        }),
    )
}

pub fn build_tweet_create_features() -> Value {
    apply_feature_overrides(
        "tweetCreate",
        json!({
            "rweb_video_screen_enabled": true,
            "creator_subscriptions_tweet_preview_api_enabled": true,
            "premium_content_api_read_enabled": false,
            "communities_web_enable_tweet_community_results_fetch": true,
            "c9s_tweet_anatomy_moderator_badge_enabled": true,
            "responsive_web_grok_analyze_button_fetch_trends_enabled": false,
            "responsive_web_grok_analyze_post_followups_enabled": false,
            "responsive_web_grok_annotations_enabled": false,
            "responsive_web_jetfuel_frame": true,
            "post_ctas_fetch_enabled": true,
            "responsive_web_grok_share_attachment_enabled": true,
            "responsive_web_edit_tweet_api_enabled": true,
            "graphql_is_translatable_rweb_tweet_is_translatable_enabled": true,
            "view_counts_everywhere_api_enabled": true,
            "longform_notetweets_consumption_enabled": true,
            "responsive_web_twitter_article_tweet_consumption_enabled": true,
            "tweet_awards_web_tipping_enabled": false,
            "responsive_web_grok_show_grok_translated_post": false,
            "responsive_web_grok_analysis_button_from_backend": true,
            "creator_subscriptions_quote_tweet_preview_enabled": false,
            "longform_notetweets_rich_text_read_enabled": true,
            "longform_notetweets_inline_media_enabled": true,
            "profile_label_improvements_pcf_label_in_post_enabled": true,
            "responsive_web_profile_redirect_enabled": false,
            "rweb_tipjar_consumption_enabled": true,
            "verified_phone_label_enabled": false,
            "articles_preview_enabled": true,
            "responsive_web_grok_community_note_auto_translation_is_enabled": false,
            "responsive_web_graphql_skip_user_profile_image_extensions_enabled": false,
            "freedom_of_speech_not_reach_fetch_enabled": true,
            "standardized_nudges_misinfo": true,
            "tweet_with_visibility_results_prefer_gql_limited_actions_policy_enabled": true,
            "responsive_web_grok_image_annotation_enabled": true,
            "responsive_web_grok_imagine_annotation_enabled": true,
            "responsive_web_graphql_timeline_navigation_enabled": true,
            "responsive_web_enhance_cards_enabled": false
        }),
    )
}

pub fn build_article_features() -> Value {
    apply_feature_overrides(
        "article",
        json!({
            "rweb_video_screen_enabled": true,
            "profile_label_improvements_pcf_label_in_post_enabled": true,
            "responsive_web_profile_redirect_enabled": true,
            "rweb_tipjar_consumption_enabled": true,
            "verified_phone_label_enabled": false,
            "creator_subscriptions_tweet_preview_api_enabled": true,
            "responsive_web_graphql_timeline_navigation_enabled": true,
            "responsive_web_graphql_exclude_directive_enabled": true,
            "responsive_web_graphql_skip_user_profile_image_extensions_enabled": false,
            "premium_content_api_read_enabled": false,
            "communities_web_enable_tweet_community_results_fetch": true,
            "c9s_tweet_anatomy_moderator_badge_enabled": true,
            "responsive_web_grok_analyze_button_fetch_trends_enabled": false,
            "responsive_web_grok_analyze_post_followups_enabled": false,
            "responsive_web_grok_annotations_enabled": false,
            "responsive_web_jetfuel_frame": true,
            "post_ctas_fetch_enabled": true,
            "responsive_web_grok_share_attachment_enabled": true,
            "articles_preview_enabled": true,
            "responsive_web_edit_tweet_api_enabled": true,
            "graphql_is_translatable_rweb_tweet_is_translatable_enabled": true,
            "view_counts_everywhere_api_enabled": true,
            "longform_notetweets_consumption_enabled": true,
            "responsive_web_twitter_article_tweet_consumption_enabled": true,
            "tweet_awards_web_tipping_enabled": false,
            "responsive_web_grok_show_grok_translated_post": false,
            "responsive_web_grok_analysis_button_from_backend": true,
            "creator_subscriptions_quote_tweet_preview_enabled": false,
            "freedom_of_speech_not_reach_fetch_enabled": true,
            "standardized_nudges_misinfo": true,
            "tweet_with_visibility_results_prefer_gql_limited_actions_policy_enabled": true,
            "longform_notetweets_rich_text_read_enabled": true,
            "longform_notetweets_inline_media_enabled": true,
            "responsive_web_grok_image_annotation_enabled": true,
            "responsive_web_grok_imagine_annotation_enabled": true,
            "responsive_web_grok_community_note_auto_translation_is_enabled": false,
            "responsive_web_enhance_cards_enabled": false
        }),
    )
}

pub fn build_tweet_detail_features() -> Value {
    let mut value = build_article_features();
    if let Some(map) = value.as_object_mut() {
        map.insert(
            "responsive_web_twitter_article_plain_text_enabled".to_owned(),
            Value::Bool(true),
        );
        map.insert(
            "responsive_web_twitter_article_seed_tweet_detail_enabled".to_owned(),
            Value::Bool(true),
        );
        map.insert(
            "responsive_web_twitter_article_seed_tweet_summary_enabled".to_owned(),
            Value::Bool(true),
        );
    }
    apply_feature_overrides("tweetDetail", value)
}

pub fn build_user_tweets_features() -> Value {
    apply_feature_overrides(
        "userTweets",
        json!({
            "rweb_video_screen_enabled": false,
            "profile_label_improvements_pcf_label_in_post_enabled": true,
            "responsive_web_profile_redirect_enabled": false,
            "rweb_tipjar_consumption_enabled": true,
            "verified_phone_label_enabled": false,
            "creator_subscriptions_tweet_preview_api_enabled": true,
            "responsive_web_graphql_timeline_navigation_enabled": true,
            "responsive_web_graphql_skip_user_profile_image_extensions_enabled": false,
            "premium_content_api_read_enabled": false,
            "communities_web_enable_tweet_community_results_fetch": true,
            "c9s_tweet_anatomy_moderator_badge_enabled": true,
            "responsive_web_grok_analyze_button_fetch_trends_enabled": false,
            "responsive_web_grok_analyze_post_followups_enabled": true,
            "responsive_web_jetfuel_frame": true,
            "post_ctas_fetch_enabled": true,
            "responsive_web_grok_share_attachment_enabled": true,
            "responsive_web_grok_annotations_enabled": false,
            "articles_preview_enabled": true,
            "responsive_web_edit_tweet_api_enabled": true,
            "graphql_is_translatable_rweb_tweet_is_translatable_enabled": true,
            "view_counts_everywhere_api_enabled": true,
            "longform_notetweets_consumption_enabled": true,
            "responsive_web_twitter_article_tweet_consumption_enabled": true,
            "tweet_awards_web_tipping_enabled": false,
            "responsive_web_grok_show_grok_translated_post": true,
            "responsive_web_grok_analysis_button_from_backend": true,
            "creator_subscriptions_quote_tweet_preview_enabled": false,
            "freedom_of_speech_not_reach_fetch_enabled": true,
            "standardized_nudges_misinfo": true,
            "tweet_with_visibility_results_prefer_gql_limited_actions_policy_enabled": true,
            "longform_notetweets_rich_text_read_enabled": true,
            "longform_notetweets_inline_media_enabled": true,
            "responsive_web_grok_image_annotation_enabled": true,
            "responsive_web_grok_imagine_annotation_enabled": true,
            "responsive_web_grok_community_note_auto_translation_is_enabled": false,
            "responsive_web_enhance_cards_enabled": false
        }),
    )
}

pub fn build_article_field_toggles() -> Value {
    json!({
        "withPayments": false,
        "withAuxiliaryUserLabels": false,
        "withArticleRichContentState": true,
        "withArticlePlainText": true,
        "withGrokAnalyze": false,
        "withDisallowedReplyControls": false
    })
}

fn build_timeline_features() -> Value {
    let mut value = build_search_features();
    if let Some(map) = value.as_object_mut() {
        map.insert("blue_business_profile_image_shape_enabled".to_owned(), Value::Bool(true));
        map.insert("responsive_web_text_conversations_enabled".to_owned(), Value::Bool(false));
        map.insert("tweetypie_unmention_optimization_enabled".to_owned(), Value::Bool(true));
        map.insert("vibe_api_enabled".to_owned(), Value::Bool(true));
        map.insert(
            "responsive_web_twitter_blue_verified_badge_is_enabled".to_owned(),
            Value::Bool(true),
        );
        map.insert("interactive_text_enabled".to_owned(), Value::Bool(true));
        map.insert(
            "longform_notetweets_richtext_consumption_enabled".to_owned(),
            Value::Bool(true),
        );
        map.insert(
            "responsive_web_media_download_video_enabled".to_owned(),
            Value::Bool(false),
        );
    }
    value
}

fn apply_feature_overrides(set_name: &str, base: Value) -> Value {
    let mut base_map = base.as_object().cloned().unwrap_or_default();
    let overrides = load_feature_overrides();
    for (key, value) in overrides.global {
        base_map.insert(key, Value::Bool(value));
    }
    if let Some(set) = overrides.sets.get(set_name) {
        for (key, value) in set {
            base_map.insert(key.clone(), Value::Bool(*value));
        }
    }
    Value::Object(base_map)
}

fn load_feature_overrides() -> FeatureOverrides {
    let mut merged = default_feature_overrides();
    if let Ok(raw) = fs::read_to_string(features_path()) {
        if let Ok(overrides) = serde_json::from_str::<FeatureOverrides>(&raw) {
            merge_overrides(&mut merged, overrides);
        }
    }
    if let Ok(raw) = std::env::var("BIRD_FEATURES_JSON") {
        if let Ok(overrides) = serde_json::from_str::<FeatureOverrides>(&raw) {
            merge_overrides(&mut merged, overrides);
        }
    }
    merged
}

fn merge_overrides(base: &mut FeatureOverrides, next: FeatureOverrides) {
    base.global.extend(next.global);
    for (set_name, overrides) in next.sets {
        base.sets.entry(set_name).or_default().extend(overrides);
    }
}

fn default_feature_overrides() -> FeatureOverrides {
    FeatureOverrides {
        global: BTreeMap::from([
            ("responsive_web_grok_annotations_enabled".to_owned(), false),
            ("post_ctas_fetch_enabled".to_owned(), true),
            ("responsive_web_graphql_exclude_directive_enabled".to_owned(), true),
        ]),
        sets: BTreeMap::from([(
            "lists".to_owned(),
            BTreeMap::from([
                ("blue_business_profile_image_shape_enabled".to_owned(), true),
                ("tweetypie_unmention_optimization_enabled".to_owned(), true),
                ("responsive_web_text_conversations_enabled".to_owned(), false),
                ("interactive_text_enabled".to_owned(), true),
                ("vibe_api_enabled".to_owned(), true),
                (
                    "responsive_web_twitter_blue_verified_badge_is_enabled".to_owned(),
                    true,
                ),
            ]),
        )]),
    }
}

fn to_json_value(overrides: &FeatureOverrides) -> Value {
    let mut root = Map::new();
    root.insert("global".to_owned(), serde_json::to_value(&overrides.global).unwrap_or(Value::Object(Map::new())));
    root.insert("sets".to_owned(), serde_json::to_value(&overrides.sets).unwrap_or(Value::Object(Map::new())));
    Value::Object(root)
}
