//! The quota of a Claude sign-in, read the way Claude Code's `/usage` reads it: the sign-in's
//! OAuth token (login Keychain on macOS, `<home>/.credentials.json` elsewhere) against
//! `api/oauth/usage`. The token goes to curl on stdin, never argv, and never leaves this module.

use std::process::Stdio;
use std::time::Duration;

use serde_json::Value;
use sha2::{Digest, Sha256};
use tokio::io::AsyncWriteExt;

const URL: &str = "https://api.anthropic.com/api/oauth/usage";
const TIMEOUT: Duration = Duration::from_secs(10);
/// The windows `/usage` shows (the API also returns product ones Codly has no label for).
const WINDOWS: &[&str] = &["five_hour", "seven_day", "seven_day_opus", "seven_day_sonnet"];

#[derive(Debug, Clone, PartialEq)]
pub struct Reading {
    pub window: String,
    pub used_fraction: f64,
    pub resets_at: Option<i64>,
    pub status: Option<String>,
}

/// Usage windows of the Claude sign-in at `home` (None = the default `~/.claude`). None = no
/// usable token (signed out, expired, an API key) or no answer.
pub async fn claude(home: Option<&str>, user_home: &str) -> Option<Vec<Reading>> {
    let creds = credentials(home, user_home).await?;
    let oauth = &creds["claudeAiOauth"];
    // ponytail: an expired token is skipped; the CLI refreshes it on its next run.
    if oauth["expiresAt"].as_i64().is_some_and(|t| t < crate::model::now_ms()) {
        return None;
    }
    let token = oauth["accessToken"].as_str().filter(|t| t.chars().all(|c| c.is_ascii_alphanumeric() || "-_.".contains(c)))?;
    let body = get(token).await?;
    Some(parse(&serde_json::from_str(&body).ok()?))
}

/// The Keychain item Claude Code keeps a config home's login in: its own name for the default
/// home, suffixed with the first 8 hex digits of sha256(home) for a `CLAUDE_CONFIG_DIR`.
fn keychain_service(home: Option<&str>) -> String {
    match home {
        None => "Claude Code-credentials".into(),
        Some(h) => format!("Claude Code-credentials-{}", &format!("{:x}", Sha256::digest(h.as_bytes()))[..8]),
    }
}

async fn credentials(home: Option<&str>, user_home: &str) -> Option<Value> {
    if cfg!(target_os = "macos") {
        let out = tokio::process::Command::new("/usr/bin/security")
            .args(["find-generic-password", "-s", &keychain_service(home), "-w"])
            .stdin(Stdio::null())
            .stderr(Stdio::null())
            .kill_on_drop(true)
            .output();
        if let Ok(Ok(o)) = tokio::time::timeout(TIMEOUT, out).await
            && o.status.success()
            && let Ok(v) = serde_json::from_slice(&o.stdout)
        {
            return Some(v);
        }
    }
    let dir = home.map_or_else(|| format!("{user_home}/.claude"), str::to_owned);
    serde_json::from_str(&tokio::fs::read_to_string(format!("{dir}/.credentials.json")).await.ok()?).ok()
}

async fn get(token: &str) -> Option<String> {
    // Values are quote-free: the URL is pinned and the token checked above.
    let config = format!(
        "url = \"{URL}\"\nproto = \"=https\"\nsilent\nfail\nmax-time = {}\nheader = \"Authorization: Bearer {token}\"\nheader = \"anthropic-beta: oauth-2025-04-20\"\n",
        TIMEOUT.as_secs()
    );
    let mut child = tokio::process::Command::new("curl")
        .args(["-K", "-"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .kill_on_drop(true)
        .spawn()
        .ok()?;
    let mut stdin = child.stdin.take()?;
    stdin.write_all(config.as_bytes()).await.ok()?;
    drop(stdin);
    let out = tokio::time::timeout(TIMEOUT + Duration::from_secs(2), child.wait_with_output()).await.ok()?.ok()?;
    out.status.success().then(|| String::from_utf8_lossy(&out.stdout).into_owned())
}

/// `{"five_hour": {"utilization": 38.0, "resets_at": "2026-…+00:00", "locked_reason": null}, …}`
fn parse(v: &Value) -> Vec<Reading> {
    WINDOWS
        .iter()
        .filter_map(|w| {
            let o = &v[*w];
            Some(Reading {
                window: w.to_string(),
                used_fraction: (o["utilization"].as_f64()? / 100.0).clamp(0.0, 1.0),
                resets_at: o["resets_at"].as_str().and_then(parse_ts),
                status: (!o["locked_reason"].is_null()).then(|| "rejected".into()),
            })
        })
        .collect()
}

/// "2026-10-30T08:09:59.841805+00:00" (or "…Z") → unix ms.
fn parse_ts(s: &str) -> Option<i64> {
    let n = |a: usize, b: usize| s.get(a..b)?.parse::<i64>().ok();
    let (y, mo, d, h, mi, se) = (n(0, 4)?, n(5, 7)?, n(8, 10)?, n(11, 13)?, n(14, 16)?, n(17, 19)?);
    let tz = s.get(19..)?.trim_start_matches(|c: char| c == '.' || c.is_ascii_digit());
    let off = match tz {
        "" | "Z" => 0,
        t => (if t.starts_with('-') { -1 } else { 1 }) * (t.get(1..3)?.parse::<i64>().ok()? * 60 + t.get(4..6)?.parse::<i64>().ok()?),
    };
    // Days since 1970-01-01 (Howard Hinnant's days_from_civil).
    let y = if mo <= 2 { y - 1 } else { y };
    let (era, yoe) = (y.div_euclid(400), y.rem_euclid(400));
    let doy = (153 * (if mo > 2 { mo - 3 } else { mo + 9 }) + 2) / 5 + d - 1;
    let days = era * 146_097 + yoe * 365 + yoe / 4 - yoe / 100 + doy - 719_468;
    Some((days * 86_400 + h * 3600 + mi * 60 + se - off * 60) * 1000)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_usage_windows() {
        let v: Value = serde_json::from_str(
            r#"{"five_hour":{"utilization":38.0,"resets_at":"2026-10-30T08:09:59.841805+00:00","locked_reason":null},
                "seven_day":{"utilization":14.0,"resets_at":"2026-11-03T14:59:59Z","locked_reason":"limit"},
                "seven_day_opus":null,"iguana_necktie":{"utilization":8.7}}"#,
        )
        .unwrap();
        let r = parse(&v);
        assert_eq!(r.len(), 2);
        assert_eq!((r[0].window.as_str(), r[0].used_fraction, r[0].resets_at, r[0].status.as_deref()), ("five_hour", 0.38, Some(1_793_347_799_000), None));
        assert_eq!((r[1].window.as_str(), r[1].status.as_deref()), ("seven_day", Some("rejected")));
        assert_eq!(parse_ts("1970-01-01T00:00:00Z"), Some(0));
        assert_eq!(parse_ts("1970-01-01T02:00:00+02:00"), Some(0));
        assert_eq!(parse_ts("2000-03-01T00:00:00Z"), Some(951_868_800_000));
        assert_eq!(parse_ts("nope"), None);
    }

    #[test]
    fn keychain_names_match_claude_code() {
        assert_eq!(keychain_service(None), "Claude Code-credentials");
        assert_eq!(keychain_service(Some("/Users/julien/.claude-perso")), "Claude Code-credentials-fd44029e");
    }
}
