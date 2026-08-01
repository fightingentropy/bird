use std::sync::{LazyLock, RwLock};

use regex::Regex;

const REDACTED: &str = "[REDACTED]";
const SECRET_ENV_KEYS: &[&str] = &["AUTH_TOKEN", "TWITTER_AUTH_TOKEN", "CT0", "TWITTER_CT0"];

static ASSIGNMENT_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r#"(?i)(\b(?:auth_token|ct0|authorization|cookie)\b[\"']?\s*[:=]\s*[\"']?)([^\s,;\"'}]+)"#,
    )
    .expect("valid secret assignment regex")
});
static PROXY_USERINFO_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"(?i)(https?://)[^\s/@:]+(?::[^\s/@]*)?@"#).expect("valid proxy userinfo regex")
});
static REGISTERED_SECRETS: LazyLock<RwLock<Vec<String>>> =
    LazyLock::new(|| RwLock::new(Vec::new()));

pub fn register_secret_for_redaction(secret: &str) {
    if secret.len() < 4 {
        return;
    }
    if let Ok(mut secrets) = REGISTERED_SECRETS.write()
        && !secrets.iter().any(|known| known == secret)
    {
        secrets.push(secret.to_owned());
    }
}

/// Redact session material from text before it reaches logs or error output.
///
/// In addition to structured cookie/header assignments, this replaces any
/// exact secret currently supplied through the supported environment variables.
pub fn redact_sensitive_text(input: &str) -> String {
    let mut redacted = ASSIGNMENT_RE
        .replace_all(input, format!("${{1}}{REDACTED}"))
        .into_owned();
    redacted = PROXY_USERINFO_RE
        .replace_all(&redacted, format!("${{1}}{REDACTED}@"))
        .into_owned();

    for key in SECRET_ENV_KEYS {
        let Some(secret) = std::env::var(key).ok().filter(|value| !value.is_empty()) else {
            continue;
        };
        redacted = redacted.replace(&secret, REDACTED);
    }
    if let Ok(secrets) = REGISTERED_SECRETS.read() {
        for secret in secrets.iter() {
            redacted = redacted.replace(secret, REDACTED);
        }
    }
    redacted
}

pub fn redacted_marker() -> &'static str {
    REDACTED
}

#[cfg(test)]
mod tests {
    use super::{redact_sensitive_text, register_secret_for_redaction};

    #[test]
    fn redacts_headers_json_and_proxy_userinfo() {
        let input = concat!(
            r#"{"auth_token":"super-secret","ct0":"csrf-secret"}"#,
            " cookie: auth_token=super-secret; ct0=csrf-secret ",
            "authorization: Bearer-token ",
            "https://proxy-user:proxy-pass@proxy.example"
        );
        let output = redact_sensitive_text(input);

        for secret in ["super-secret", "csrf-secret", "Bearer-token", "proxy-pass"] {
            assert!(!output.contains(secret));
        }
        assert!(output.contains("[REDACTED]"));
    }

    #[test]
    fn redacts_registered_unstructured_secrets() {
        register_secret_for_redaction("stdin-only-session-secret");
        let output = redact_sensitive_text("failure contained stdin-only-session-secret verbatim");
        assert!(!output.contains("stdin-only-session-secret"));
        assert!(output.contains("[REDACTED]"));
    }
}
