use std::time::Duration;

use anyhow::Context;
use clap::Parser;
use sweet_cookie::{
    browsers_for_cli, get_cookies, parse_mode, CookieHeaderOptions, CookieHeaderSort,
    GetCookiesOptions,
};

#[derive(Debug, Parser)]
#[command(name = "sweet-cookie-diagnose")]
#[command(about = "Inspect browser cookies and emitted Cookie headers")]
struct Args {
    #[arg(long)]
    url: String,
    #[arg(long = "origin")]
    origins: Vec<String>,
    #[arg(long = "name")]
    names: Vec<String>,
    #[arg(long = "browser")]
    browsers: Vec<String>,
    #[arg(long)]
    profile: Option<String>,
    #[arg(long = "chrome-profile")]
    chrome_profile: Option<String>,
    #[arg(long = "edge-profile")]
    edge_profile: Option<String>,
    #[arg(long = "firefox-profile")]
    firefox_profile: Option<String>,
    #[arg(long = "safari-cookies-file")]
    safari_cookies_file: Option<String>,
    #[arg(long = "inline-cookies-file")]
    inline_cookies_file: Option<String>,
    #[arg(long = "inline-cookies-json")]
    inline_cookies_json: Option<String>,
    #[arg(long = "inline-cookies-base64")]
    inline_cookies_base64: Option<String>,
    #[arg(long = "include-expired", default_value_t = false)]
    include_expired: bool,
    #[arg(long = "timeout-ms")]
    timeout_ms: Option<u64>,
    #[arg(long)]
    mode: Option<String>,
    #[arg(long = "dedupe-by-name", default_value_t = false)]
    dedupe_by_name: bool,
}

fn main() -> anyhow::Result<()> {
    let args = Args::parse();
    let result = get_cookies(GetCookiesOptions {
        url: args.url,
        origins: args.origins,
        names: args.names,
        browsers: browsers_for_cli(&args.browsers).context("failed to parse browsers")?,
        profile: args.profile,
        chrome_profile: args.chrome_profile,
        edge_profile: args.edge_profile,
        firefox_profile: args.firefox_profile,
        safari_cookies_file: args.safari_cookies_file.map(Into::into),
        include_expired: args.include_expired,
        timeout: args.timeout_ms.map(Duration::from_millis),
        debug: false,
        mode: parse_mode(args.mode.as_deref())?,
        inline_cookies_file: args.inline_cookies_file.map(Into::into),
        inline_cookies_json: args.inline_cookies_json,
        inline_cookies_base64: args.inline_cookies_base64,
    })?;

    for warning in &result.warnings {
        eprintln!("warning: {warning}");
    }
    println!("{}", serde_json::to_string_pretty(&result.cookies)?);
    println!();
    println!(
        "{}",
        sweet_cookie::to_cookie_header(
            &result.cookies,
            CookieHeaderOptions {
                dedupe_by_name: args.dedupe_by_name,
                sort: CookieHeaderSort::Name,
            }
        )
    );
    Ok(())
}
