//! Track 19 text-to-scene: a description goes to Claude (Messages API, raw HTTP; Rust has no
//! official SDK) with the `Settings` source as its schema, and the JSON that comes back goes
//! through `validate()`. A rejected scene is sent back with the error for another try.
//!
//! The key comes from `ANTHROPIC_API_KEY`. The system prompt (scene.rs, two example files) is
//! the same on every call, so it is marked for prompt caching: retries and later prompts
//! read it from the cache.
use crate::scene::Settings;
use anyhow::{Context, Result, bail};
use serde_json::{Value, json};

const MODEL: &str = "claude-opus-5-5";
const URL: &str = "https://api.anthropic.com/v1/messages";
/// Tries after the first when the scene does not validate.
const REPAIRS: usize = 2;
/// Retries of one request on 429, 5xx (529 overloaded included) and connection errors.
const RETRIES: u32 = 3;

fn system() -> String {
    format!(
        "You design scenes for Cornell RTX, a path tracer. Reply with one scene file as JSON and nothing else: \
         {{\"format\": 1, \"title\": ..., \"description\": ..., \"tags\": [...], \"settings\": {{...}}}}. \
         `settings` is the serde form of the Rust `Settings` struct below; omitted fields take their defaults, \
         so set only what the scene needs. Every value must pass `Settings::validate()` (the ranges are in its \
         source). Units: the Cornell room spans -1..1; the floor is at y = -1; an object's world size is \
         size × 0.35 (× stretch on y, × scale_xz on x and z). Keep it under 200 objects.\n\n\
         <scene_rs>\n{}\n</scene_rs>\n\n<example>\n{}\n</example>\n\n<example>\n{}\n</example>",
        include_str!("scene.rs"),
        include_str!("../scenes/builtin/desert-monolith.json"),
        include_str!("../scenes/builtin/cornell-caustic.json"),
    )
}

/// Sends one request body and returns the response JSON (swapped out in tests).
pub type Transport<'a> = dyn FnMut(&Value) -> Result<Value> + 'a;

/// The real transport: ureq with retries and backoff on transient failures.
pub fn http(key: String) -> impl FnMut(&Value) -> Result<Value> {
    // native-tls like the asset downloader, but with a long read timeout: a scene can take
    // Claude a few minutes.
    let agent = native_tls::TlsConnector::new().map(|tls| {
        ureq::AgentBuilder::new()
            .tls_connector(std::sync::Arc::new(tls))
            .timeout_connect(std::time::Duration::from_secs(15))
            .timeout(std::time::Duration::from_secs(600))
            .build()
    });
    move |body: &Value| {
        let agent = agent.as_ref().map_err(|e| anyhow::anyhow!("TLS: {e}"))?;
        let mut wait = std::time::Duration::from_secs(2);
        for attempt in 0..=RETRIES {
            let r = agent
                .post(URL)
                .set("x-api-key", &key)
                .set("anthropic-version", "2026-10-21")
                .set("anthropic-beta", "server-side-fallback-2026-07-01")
                .set("content-type", "application/json")
                .send_string(&body.to_string());
            match r {
                Ok(resp) => return Ok(serde_json::from_reader(resp.into_reader())?),
                Err(ureq::Error::Status(code, resp))
                    if attempt < RETRIES && (code == 429 || code >= 500) =>
                {
                    if let Some(s) = resp
                        .header("retry-after")
                        .and_then(|v| v.parse::<u64>().ok())
                    {
                        wait = std::time::Duration::from_secs(s.min(60));
                    }
                }
                Err(ureq::Error::Status(code, resp)) => {
                    let text = resp.into_string().unwrap_or_default();
                    bail!("Claude API error {code}: {text}");
                }
                Err(ureq::Error::Transport(_)) if attempt < RETRIES => {}
                Err(e) => return Err(e.into()),
            }
            std::thread::sleep(wait);
            wait *= 2;
        }
        unreachable!()
    }
}

/// The scene JSON in a reply: the text between the first `{` and the last `}` (the model is
/// asked for bare JSON, but a code fence around it is tolerated).
fn extract(text: &str) -> Option<&str> {
    let a = text.find('{')?;
    let b = text.rfind('}')?;
    (b > a).then(|| &text[a..=b])
}

/// Turns `prompt` into validated settings, repairing up to `REPAIRS` times.
pub fn generate(prompt: &str, send: &mut Transport) -> Result<Settings> {
    let mut messages = vec![json!({"role": "user", "content": prompt})];
    let mut last = String::new();
    for _ in 0..=REPAIRS {
        let body = json!({
            "model": MODEL,
            "max_tokens": 32000,
            "output_config": {"effort": "medium"},
            "fallbacks": "default",
            "system": [{"type": "text", "text": system(), "cache_control": {"type": "ephemeral"}}],
            "messages": messages,
        });
        let reply = send(&body)?;
        match reply["stop_reason"].as_str() {
            Some("refusal") => bail!(
                "Claude declined: {}",
                reply["stop_details"]["explanation"]
                    .as_str()
                    .unwrap_or("no reason given")
            ),
            Some("max_tokens") => bail!("Claude's reply was cut off (the scene was too long)"),
            _ => {}
        }
        let blocks = reply["content"]
            .as_array()
            .filter(|c| !c.is_empty())
            .context("Claude sent an empty reply")?;
        let text: String = blocks
            .iter()
            .filter(|b| b["type"] == "text")
            .filter_map(|b| b["text"].as_str())
            .collect();
        let result = extract(&text)
            .context("no JSON object in the reply")
            .and_then(|j| Ok(serde_json::from_str::<Value>(j)?))
            .and_then(crate::scenes::parse)
            .and_then(|mut s| {
                s.validate()?;
                Ok(s)
            });
        match result {
            Ok(s) => return Ok(s),
            Err(e) => {
                last = format!("{e:#}");
                // Send the whole reply back (thinking blocks included, unchanged) and ask
                // for a fix.
                messages.push(json!({"role": "assistant", "content": reply["content"]}));
                messages.push(json!({"role": "user", "content": format!(
                    "That scene was rejected: {last}. Reply with the corrected scene file as JSON only."
                )}));
            }
        }
    }
    bail!("Claude's scene did not validate: {last}")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn reply(text: &str) -> Value {
        json!({"stop_reason": "end_turn", "content": [
            {"type": "thinking", "thinking": "", "signature": "x"},
            {"type": "text", "text": text}
        ]})
    }

    #[test]
    fn an_invalid_scene_is_sent_back_and_the_fix_accepted() {
        let mut calls = vec![];
        let mut answers = vec![
            reply(
                r#"```json
{"format": 1, "title": "Bad", "settings": {"spp": 1000}}
```"#,
            ),
            reply(r#"{"format": 1, "title": "Two boxes", "settings": {"spp": 4}}"#),
        ]
        .into_iter();
        let mut send = |body: &Value| {
            calls.push(body.clone());
            Ok(answers.next().unwrap())
        };
        let s = generate("two boxes", &mut send).unwrap();
        assert_eq!(s.scene_name, "Two boxes");
        assert_eq!(s.spp, 4);
        assert_eq!(calls.len(), 2);
        let second = calls[1]["messages"].as_array().unwrap();
        assert_eq!(second.len(), 3, "prompt, rejected reply, the error");
        assert!(second[2]["content"].as_str().unwrap().contains("rejected"));
        assert_eq!(calls[0]["model"], MODEL);
    }

    #[test]
    fn refusals_and_hopeless_replies_are_errors() {
        let mut send = |_: &Value| {
            Ok(json!({"stop_reason": "refusal", "content": [],
                "stop_details": {"type": "refusal", "explanation": "no"}}))
        };
        assert!(generate("x", &mut send).is_err());
        let mut n = 0;
        let mut send = |_: &Value| {
            n += 1;
            Ok(reply("no json here"))
        };
        assert!(generate("x", &mut send).is_err());
        assert_eq!(n, REPAIRS + 1);
    }
}
