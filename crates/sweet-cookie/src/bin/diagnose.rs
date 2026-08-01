use std::fs::File;
use std::io::{self, Read};
use std::path::PathBuf;
use std::time::Duration;

use anyhow::Context;
use clap::Parser;
use sweet_cookie::{
    CookieHeaderOptions, CookieHeaderSort, GetCookiesOptions, browsers_for_cli, get_cookies,
    parse_mode, to_redacted_cookie_header,
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
    #[arg(long = "inline-cookies-stdin", conflicts_with = "inline_cookies_fd")]
    inline_cookies_stdin: bool,
    #[arg(long = "inline-cookies-fd", conflicts_with = "inline_cookies_stdin")]
    inline_cookies_fd: Option<u32>,
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
    let inline_payload = read_inline_payload(&args)?;
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
        inline_cookies_json: inline_payload,
        inline_cookies_base64: None,
    })?;

    for warning in &result.warnings {
        eprintln!("warning: {warning}");
    }
    println!("{}", serde_json::to_string_pretty(&result.cookies)?);
    println!();
    println!(
        "{}",
        to_redacted_cookie_header(
            &result.cookies,
            CookieHeaderOptions {
                dedupe_by_name: args.dedupe_by_name,
                sort: CookieHeaderSort::Name,
            }
        )
    );
    Ok(())
}

fn read_inline_payload(args: &Args) -> anyhow::Result<Option<String>> {
    const MAX_SECRET_INPUT_BYTES: u64 = 1024 * 1024;
    let mut input: Option<Box<dyn Read>> = if args.inline_cookies_stdin {
        Some(Box::new(io::stdin()))
    } else if let Some(fd) = args.inline_cookies_fd {
        #[cfg(unix)]
        {
            let path = PathBuf::from(format!("/dev/fd/{fd}"));
            Some(Box::new(File::open(path)?))
        }
        #[cfg(not(unix))]
        {
            anyhow::bail!("--inline-cookies-fd is supported only on Unix platforms");
        }
    } else {
        None
    };

    let Some(input) = input.as_mut() else {
        return Ok(None);
    };
    let mut payload = String::new();
    input
        .take(MAX_SECRET_INPUT_BYTES + 1)
        .read_to_string(&mut payload)?;
    if payload.len() as u64 > MAX_SECRET_INPUT_BYTES {
        anyhow::bail!("inline cookie input exceeds the 1 MiB safety limit");
    }
    Ok(Some(payload))
}
