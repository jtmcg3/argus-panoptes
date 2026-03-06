//! Telegram bridge for the coordinator.
//!
//! Long-polls Telegram Bot API, forwards user messages to the coordinator's
//! JSON-RPC endpoint, and sends replies back to Telegram chat(s).

use anyhow::{Context, Result, anyhow};
use reqwest::Client;
use serde::Deserialize;
use serde_json::json;
use std::collections::HashSet;
use std::time::Duration;
use tracing::{debug, error, info, warn};

const TELEGRAM_MAX_MESSAGE_LENGTH: usize = 4096;
const TELEGRAM_CHUNK_OVERHEAD: usize = 30;

#[derive(Debug, Clone)]
struct TelegramBridgeConfig {
    bot_token: String,
    coordinator_url: String,
    allowed_chat_ids: HashSet<i64>,
    poll_timeout_seconds: u64,
    poll_interval_ms: u64,
    coordinator_timeout_ms: u64,
}

impl TelegramBridgeConfig {
    fn from_env_and_file() -> Result<Self> {
        let path =
            std::env::var("PANOPTES_CONFIG").unwrap_or_else(|_| "config/default.toml".into());
        let text = std::fs::read_to_string(&path).unwrap_or_default();
        let table: toml::Value = text
            .parse()
            .unwrap_or(toml::Value::Table(Default::default()));

        let token_from_file = table
            .get("telegram")
            .and_then(|t| t.get("bot_token"))
            .and_then(|v| v.as_str())
            .map(str::to_string);
        let bot_token = std::env::var("TELEGRAM_BOT_TOKEN")
            .ok()
            .or(token_from_file)
            .filter(|s| !s.trim().is_empty())
            .ok_or_else(|| {
                anyhow!(
                    "Telegram bot token missing. Set TELEGRAM_BOT_TOKEN or [telegram].bot_token"
                )
            })?;

        let coordinator_url = std::env::var("TELEGRAM_COORDINATOR_URL")
            .ok()
            .or_else(|| {
                table
                    .get("telegram")
                    .and_then(|t| t.get("coordinator_url"))
                    .and_then(|v| v.as_str())
                    .map(str::to_string)
            })
            .unwrap_or_else(|| {
                let port = table
                    .get("port")
                    .and_then(|v| v.as_integer())
                    .unwrap_or(18080);
                format!("http://localhost:{port}/")
            });
        let coordinator_url = normalize_url(&coordinator_url);

        let allowed_chat_ids =
            parse_allowed_chat_ids(std::env::var("TELEGRAM_ALLOWED_CHAT_IDS").ok())
                .or_else(|| {
                    table
                        .get("telegram")
                        .and_then(|t| t.get("allowed_chat_ids"))
                        .and_then(|v| v.as_array())
                        .map(|arr| {
                            arr.iter()
                                .filter_map(|v| v.as_integer())
                                .filter_map(|n| i64::try_from(n).ok())
                                .collect::<HashSet<_>>()
                        })
                })
                .unwrap_or_default();

        let poll_timeout_seconds = std::env::var("TELEGRAM_POLL_TIMEOUT_SECONDS")
            .ok()
            .and_then(|v| v.parse::<u64>().ok())
            .or_else(|| {
                table
                    .get("telegram")
                    .and_then(|t| t.get("poll_timeout_seconds"))
                    .and_then(|v| v.as_integer())
                    .and_then(|v| u64::try_from(v).ok())
            })
            .unwrap_or(30);

        let poll_interval_ms = std::env::var("TELEGRAM_POLL_INTERVAL_MS")
            .ok()
            .and_then(|v| v.parse::<u64>().ok())
            .or_else(|| {
                table
                    .get("telegram")
                    .and_then(|t| t.get("poll_interval_ms"))
                    .and_then(|v| v.as_integer())
                    .and_then(|v| u64::try_from(v).ok())
            })
            .unwrap_or(1000);

        let coordinator_timeout_ms = std::env::var("TELEGRAM_COORDINATOR_TIMEOUT_MS")
            .ok()
            .and_then(|v| v.parse::<u64>().ok())
            .or_else(|| {
                table
                    .get("telegram")
                    .and_then(|t| t.get("coordinator_timeout_ms"))
                    .and_then(|v| v.as_integer())
                    .and_then(|v| u64::try_from(v).ok())
            })
            .unwrap_or(180000);

        Ok(Self {
            bot_token,
            coordinator_url,
            allowed_chat_ids,
            poll_timeout_seconds,
            poll_interval_ms,
            coordinator_timeout_ms,
        })
    }
}

#[derive(Debug, Deserialize)]
struct TelegramGetUpdatesResponse {
    ok: bool,
    result: Vec<TelegramUpdate>,
    description: Option<String>,
}

#[derive(Debug, Deserialize)]
struct TelegramUpdate {
    update_id: i64,
    message: Option<TelegramMessage>,
}

#[derive(Debug, Deserialize)]
struct TelegramMessage {
    message_id: i64,
    chat: TelegramChat,
    from: Option<TelegramUser>,
    text: Option<String>,
}

#[derive(Debug, Deserialize)]
struct TelegramChat {
    id: i64,
}

#[derive(Debug, Deserialize)]
struct TelegramUser {
    id: i64,
    is_bot: Option<bool>,
}

fn normalize_url(url: &str) -> String {
    if url.ends_with('/') {
        url.to_string()
    } else {
        format!("{url}/")
    }
}

fn parse_allowed_chat_ids(raw: Option<String>) -> Option<HashSet<i64>> {
    let raw = raw?;
    let mut out = HashSet::new();
    for part in raw.split(',') {
        let part = part.trim();
        if part.is_empty() {
            continue;
        }
        if let Ok(id) = part.parse::<i64>() {
            out.insert(id);
        }
    }
    Some(out)
}

fn split_message_for_telegram(message: &str) -> Vec<String> {
    if message.chars().count() <= TELEGRAM_MAX_MESSAGE_LENGTH {
        return vec![message.to_string()];
    }

    let mut chunks = Vec::new();
    let mut remaining = message;
    let chunk_limit = TELEGRAM_MAX_MESSAGE_LENGTH - TELEGRAM_CHUNK_OVERHEAD;

    while !remaining.is_empty() {
        if remaining.chars().count() <= TELEGRAM_MAX_MESSAGE_LENGTH {
            chunks.push(remaining.to_string());
            break;
        }

        let hard_split = remaining
            .char_indices()
            .nth(chunk_limit)
            .map_or(remaining.len(), |(idx, _)| idx);

        let chunk_end = if hard_split == remaining.len() {
            hard_split
        } else {
            let search_area = &remaining[..hard_split];
            if let Some(pos) = search_area.rfind('\n') {
                if search_area[..pos].chars().count() >= chunk_limit / 2 {
                    pos + 1
                } else {
                    search_area.rfind(' ').unwrap_or(hard_split) + 1
                }
            } else if let Some(pos) = search_area.rfind(' ') {
                pos + 1
            } else {
                hard_split
            }
        };

        chunks.push(remaining[..chunk_end].to_string());
        remaining = &remaining[chunk_end..];
    }

    chunks
}

async fn telegram_send_chat_action(
    http: &Client,
    token: &str,
    chat_id: i64,
    action: &str,
) -> Result<()> {
    let url = format!("https://api.telegram.org/bot{token}/sendChatAction");
    let body = json!({
        "chat_id": chat_id,
        "action": action
    });
    let resp = http.post(url).json(&body).send().await?;
    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        return Err(anyhow!("sendChatAction failed: {status} {body}"));
    }
    Ok(())
}

async fn telegram_send_text(http: &Client, token: &str, chat_id: i64, text: &str) -> Result<()> {
    let url = format!("https://api.telegram.org/bot{token}/sendMessage");

    for chunk in split_message_for_telegram(text) {
        let body = json!({
            "chat_id": chat_id,
            "text": chunk,
        });
        let resp = http.post(&url).json(&body).send().await?;
        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            return Err(anyhow!("sendMessage failed: {status} {body}"));
        }
    }

    Ok(())
}

async fn coordinator_send_message(
    http: &Client,
    coordinator_url: &str,
    text: &str,
) -> Result<String> {
    let payload = json!({
        "jsonrpc": "2.0",
        "id": format!("tg-{}", uuid::Uuid::new_v4()),
        "method": "message/send",
        "params": {
            "message": {
                "role": "user",
                "parts": [{ "type": "text", "text": text }]
            }
        }
    });

    let resp = http.post(coordinator_url).json(&payload).send().await?;
    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        return Err(anyhow!("Coordinator request failed: {status} {body}"));
    }

    let body: serde_json::Value = resp.json().await?;
    if let Some(err) = body.get("error") {
        let message = err
            .get("message")
            .and_then(|m| m.as_str())
            .unwrap_or("unknown coordinator error");
        return Err(anyhow!("Coordinator error: {message}"));
    }

    let text = body
        .get("result")
        .and_then(|r| r.get("artifacts"))
        .and_then(|a| a.as_array())
        .and_then(|artifacts| artifacts.first())
        .and_then(|artifact| artifact.get("parts"))
        .and_then(|parts| parts.as_array())
        .and_then(|parts| {
            parts
                .iter()
                .filter_map(|p| p.get("text").and_then(|t| t.as_str()))
                .next()
        })
        .unwrap_or("No response from coordinator")
        .to_string();

    Ok(text)
}

async fn telegram_get_updates(
    http: &Client,
    token: &str,
    offset: Option<i64>,
    timeout_seconds: u64,
) -> Result<Vec<TelegramUpdate>> {
    let url = format!("https://api.telegram.org/bot{token}/getUpdates");
    let body = json!({
        "offset": offset,
        "timeout": timeout_seconds,
        "allowed_updates": ["message"],
    });

    let resp = http.post(url).json(&body).send().await?;
    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        return Err(anyhow!("getUpdates failed: {status} {body}"));
    }

    let parsed: TelegramGetUpdatesResponse = resp.json().await?;
    if !parsed.ok {
        let detail = parsed
            .description
            .unwrap_or_else(|| "unknown telegram error".to_string());
        return Err(anyhow!("Telegram API returned ok=false: {detail}"));
    }

    Ok(parsed.result)
}

fn is_allowed_chat(config: &TelegramBridgeConfig, chat_id: i64) -> bool {
    if config.allowed_chat_ids.is_empty() {
        return true;
    }
    config.allowed_chat_ids.contains(&chat_id)
}

fn help_text() -> &'static str {
    "Send any message and I will route it through panoptes-coordinator.\n\nCommands:\n/start - show this help\n/help - show this help"
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter("info,panoptes=debug")
        .init();

    let config = TelegramBridgeConfig::from_env_and_file()?;
    info!(
        coordinator_url = %config.coordinator_url,
        allowed_chat_ids = config.allowed_chat_ids.len(),
        "Starting Telegram bridge"
    );

    let telegram_http = Client::builder()
        .timeout(Duration::from_secs(config.poll_timeout_seconds + 10))
        .build()
        .context("failed to build Telegram HTTP client")?;
    let coordinator_http = Client::builder()
        .timeout(Duration::from_millis(config.coordinator_timeout_ms))
        .build()
        .context("failed to build coordinator HTTP client")?;

    let mut offset: Option<i64> = None;

    loop {
        let updates = match telegram_get_updates(
            &telegram_http,
            &config.bot_token,
            offset,
            config.poll_timeout_seconds,
        )
        .await
        {
            Ok(updates) => updates,
            Err(e) => {
                warn!(error = %e, "Telegram polling failed");
                tokio::time::sleep(Duration::from_millis(config.poll_interval_ms)).await;
                continue;
            }
        };

        if updates.is_empty() {
            tokio::time::sleep(Duration::from_millis(config.poll_interval_ms)).await;
            continue;
        }

        for update in updates {
            offset = Some(update.update_id + 1);
            let Some(message) = update.message else {
                continue;
            };

            let chat_id = message.chat.id;
            if !is_allowed_chat(&config, chat_id) {
                warn!(chat_id, "Ignoring message from unauthorized chat");
                continue;
            }

            if message
                .from
                .as_ref()
                .and_then(|u| u.is_bot)
                .unwrap_or(false)
            {
                debug!(chat_id, "Ignoring bot-authored message");
                continue;
            }

            let Some(text) = message.text else {
                debug!(
                    chat_id,
                    message_id = message.message_id,
                    "Ignoring non-text Telegram message"
                );
                continue;
            };
            let text = text.trim();
            if text.is_empty() {
                continue;
            }

            info!(
                chat_id,
                from_user = message.from.as_ref().map(|u| u.id),
                "Incoming Telegram message"
            );

            if text == "/start" || text == "/help" {
                if let Err(e) =
                    telegram_send_text(&telegram_http, &config.bot_token, chat_id, help_text())
                        .await
                {
                    warn!(chat_id, error = %e, "Failed sending Telegram help response");
                }
                continue;
            }

            let _ = telegram_send_chat_action(&telegram_http, &config.bot_token, chat_id, "typing")
                .await;

            let reply =
                match coordinator_send_message(&coordinator_http, &config.coordinator_url, text)
                    .await
                {
                    Ok(reply) => reply,
                    Err(e) => {
                        error!(chat_id, error = %e, "Coordinator request failed");
                        format!("Coordinator error: {e}")
                    }
                };

            if let Err(e) =
                telegram_send_text(&telegram_http, &config.bot_token, chat_id, &reply).await
            {
                warn!(chat_id, error = %e, "Failed sending Telegram reply");
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn split_short_message() {
        let msg = "hello";
        let chunks = split_message_for_telegram(msg);
        assert_eq!(chunks.len(), 1);
        assert_eq!(chunks[0], "hello");
    }

    #[test]
    fn split_long_message() {
        let msg = "a".repeat(10_000);
        let chunks = split_message_for_telegram(&msg);
        assert!(chunks.len() > 1);
        assert!(
            chunks
                .iter()
                .all(|c| c.chars().count() <= TELEGRAM_MAX_MESSAGE_LENGTH)
        );
        assert_eq!(chunks.concat(), msg);
    }
}
