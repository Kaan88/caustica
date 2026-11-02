//! Track 19 text-to-scene: a description goes to a language model with the `Settings` source
//! as its schema, and the JSON that comes back goes through `validate()`. A rejected scene is
//! sent back with the error for another try.
//!
//! Backends (`Provider`, chosen in Menu → Text-to-scene model, kept in prefs.json):
//! - the Claude Code CLI (`claude -p`), which uses the CLI's own login (a Claude
//!   subscription works, no API key);
//! - the Claude API (Messages API over raw HTTP; Rust has no official SDK), key from
//!   `ANTHROPIC_API_KEY` or the menu;
//! - ChatGPT (OpenAI chat completions), key from `OPENAI_API_KEY` or the menu;
//! - a custom endpoint, OpenAI- or Anthropic-compatible (Ollama, LM Studio, llama.cpp,
//!   vLLM and similar local servers), key optional.
//!
//! Keys typed in the menu live for the session only; prefs.json never holds them.
use crate::scene::Settings;
use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

const CLAUDE_MODEL: &str = "claude-opus-5-5";
const ANTHROPIC_URL: &str = "https://api.anthropic.com/v1/messages";
const OPENAI_URL: &str = "https://api.openai.com/v1";
/// Tries after the first when the scene does not validate.
const REPAIRS: usize = 2;
/// Retries of one request on 429, 5xx (529 overloaded included) and connection errors.
const RETRIES: u32 = 3;

#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum Provider {
    #[default]
    ClaudeCli,
    ClaudeApi,
    OpenAi,
    Custom,
}
impl Provider {
    pub const ALL: [Provider; 4] = [Self::ClaudeCli, Self::ClaudeApi, Self::OpenAi, Self::Custom];
    pub fn label(self) -> &'static str {
        match self {
            Self::ClaudeCli => "Claude Code CLI (your Claude login)",
            Self::ClaudeApi => "Claude API (API key)",
            Self::OpenAi => "ChatGPT (OpenAI API key)",
            Self::Custom => "Custom endpoint (local models)",
        }
    }
    /// The environment variable a key falls back to.
    pub fn key_env(self) -> Option<&'static str> {
        match self {
            Self::ClaudeApi => Some("ANTHROPIC_API_KEY"),
            Self::OpenAi => Some("OPENAI_API_KEY"),
            _ => None,
        }
    }
}

/// The wire format of a custom endpoint.
#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum Protocol {
    /// `POST {base}/chat/completions`.
    #[default]
    OpenAi,
    /// `POST {base}/messages` (or `{base}/v1/messages` when the base has no `/v1`).
    Anthropic,
}

/// The model settings kept in prefs.json (no keys).
#[derive(Serialize, Deserialize, Clone, PartialEq, Debug)]
#[serde(default)]
pub struct AiConfig {
    pub provider: Provider,
    /// Empty: the provider's default (the CLI's own choice, `claude-opus-5-5` on the API).
    /// ChatGPT and custom endpoints need one.
    pub model: String,
    /// Custom endpoint base URL, e.g. `http://localhost:11434/v1` (Ollama).
    pub base_url: String,
    pub protocol: Protocol,
}
impl Default for AiConfig {
    fn default() -> Self {
        Self {
            provider: Provider::ClaudeCli,
            model: String::new(),
            base_url: "http://localhost:11434/v1".into(),
            protocol: Protocol::OpenAi,
        }
    }
}
impl AiConfig {
    /// Why a request cannot be sent yet (shown under the Generate button), given the key
    /// typed in the menu.
    pub fn missing(&self, key: &str) -> Option<String> {
        let model = self.model.trim().is_empty();
        match self.provider {
            Provider::ClaudeCli => None,
            Provider::ClaudeApi | Provider::OpenAi => {
                let env = self.provider.key_env().unwrap();
                if key.trim().is_empty() && env_key(env).is_none() {
                    return Some(format!(
                        "Set {env} or type a key in Menu > Text-to-scene model"
                    ));
                }
                (self.provider == Provider::OpenAi && model)
                    .then(|| "Choose a model in Menu > Text-to-scene model".into())
            }
            Provider::Custom if self.base_url.trim().is_empty() => {
                Some("Set the endpoint URL in Menu > Text-to-scene model".into())
            }
            Provider::Custom => {
                model.then(|| "Choose a model in Menu > Text-to-scene model".into())
            }
        }
    }
    /// A short name for messages ("Claude made …").
    pub fn who(&self) -> String {
        match self.provider {
            Provider::ClaudeCli | Provider::ClaudeApi => "Claude".into(),
            Provider::OpenAi => "ChatGPT".into(),
            Provider::Custom => self.model.trim().to_string(),
        }
    }
}

fn env_key(name: &str) -> Option<String> {
    std::env::var(name).ok().filter(|k| !k.trim().is_empty())
}

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

/// One reply: its text, and the assistant content to send back on a repair (Anthropic's
/// content blocks unchanged, thinking included; a plain string elsewhere).
pub struct Turn {
    pub text: String,
    pub raw: Value,
}

/// Sends the conversation so far (`{"role", "content"}` messages, the system prompt added by
/// the backend) and returns the reply. Swapped out in tests.
pub type Transport<'a> = dyn FnMut(&[Value]) -> Result<Turn> + 'a;

/// A ready backend (`transport`).
pub type Backend = Box<dyn FnMut(&[Value]) -> Result<Turn> + Send>;

/// The backend for `config`; `key` is the one typed in the menu (empty: the environment).
pub fn transport(config: &AiConfig, key: &str) -> Result<Backend> {
    if let Some(why) = config.missing(key) {
        bail!("{why}");
    }
    let key = Some(key.trim().to_string())
        .filter(|k| !k.is_empty())
        .or_else(|| config.provider.key_env().and_then(env_key));
    let model = config.model.trim().to_string();
    Ok(match config.provider {
        Provider::ClaudeCli => Box::new(cli(model)),
        Provider::ClaudeApi => {
            let model = if model.is_empty() {
                CLAUDE_MODEL.into()
            } else {
                model
            };
            Box::new(anthropic(ANTHROPIC_URL.into(), key, model, true))
        }
        Provider::OpenAi => Box::new(openai(OPENAI_URL.into(), key, model)),
        Provider::Custom => {
            let base = config.base_url.trim().trim_end_matches('/').to_string();
            match config.protocol {
                Protocol::OpenAi => Box::new(openai(base, key, model)),
                Protocol::Anthropic => {
                    let url = if base.ends_with("/v1") {
                        format!("{base}/messages")
                    } else {
                        format!("{base}/v1/messages")
                    };
                    Box::new(anthropic(url, key, model, false))
                }
            }
        }
    })
}

/// Posts JSON with retries and backoff on transient failures.
fn poster() -> impl FnMut(&str, &[(&str, String)], &Value) -> Result<Value> {
    // native-tls like the asset downloader, but with a long read timeout: a scene can take a
    // model a few minutes (a local one longer).
    let agent = native_tls::TlsConnector::new().map(|tls| {
        ureq::AgentBuilder::new()
            .tls_connector(std::sync::Arc::new(tls))
            .timeout_connect(std::time::Duration::from_secs(15))
            .timeout(std::time::Duration::from_secs(900))
            .build()
    });
    move |url: &str, headers: &[(&str, String)], body: &Value| {
        let agent = agent.as_ref().map_err(|e| anyhow::anyhow!("TLS: {e}"))?;
        let mut wait = std::time::Duration::from_secs(2);
        for attempt in 0..=RETRIES {
            let mut req = agent.post(url).set("content-type", "application/json");
            for (k, v) in headers {
                req = req.set(k, v);
            }
            match req.send_string(&body.to_string()) {
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
                    bail!("API error {code}: {text}");
                }
                // Connection failures are retried; a read timeout is not (the model already
                // had the whole timeout: retrying a slow local model would take an hour).
                Err(ureq::Error::Transport(t))
                    if attempt < RETRIES && t.kind() != ureq::ErrorKind::Io => {}
                Err(e) => return Err(e.into()),
            }
            std::thread::sleep(wait);
            wait *= 2;
        }
        unreachable!()
    }
}

/// The Messages API. `official`: api.anthropic.com, with prompt caching, effort and refusal
/// fallbacks; compatible servers get the plain fields only.
fn anthropic(
    url: String,
    key: Option<String>,
    model: String,
    official: bool,
) -> impl FnMut(&[Value]) -> Result<Turn> {
    let mut post = poster();
    move |messages: &[Value]| {
        let mut headers = vec![("anthropic-version", "2026-10-21".to_string())];
        if let Some(k) = &key {
            headers.push(("x-api-key", k.clone()));
            if !official {
                // Compatible proxies differ: some read the bearer header instead.
                headers.push(("authorization", format!("Bearer {k}")));
            }
        }
        let body = if official {
            headers.push(("anthropic-beta", "server-side-fallback-2026-07-01".into()));
            json!({
                "model": model,
                "max_tokens": 32000,
                "output_config": {"effort": "medium"},
                "fallbacks": "default",
                // The system prompt is the same on every call: retries and later prompts
                // read it from the cache.
                "system": [{"type": "text", "text": system(), "cache_control": {"type": "ephemeral"}}],
                "messages": messages,
            })
        } else {
            json!({"model": model, "max_tokens": 32000, "system": system(), "messages": messages})
        };
        let reply = post(&url, &headers, &body)?;
        match reply["stop_reason"].as_str() {
            Some("refusal") => bail!(
                "the model declined: {}",
                reply["stop_details"]["explanation"]
                    .as_str()
                    .unwrap_or("no reason given")
            ),
            Some("max_tokens") => bail!("the reply was cut off (the scene was too long)"),
            _ => {}
        }
        let blocks = reply["content"]
            .as_array()
            .filter(|c| !c.is_empty())
            .context("the model sent an empty reply")?;
        let text = blocks
            .iter()
            .filter(|b| b["type"] == "text")
            .filter_map(|b| b["text"].as_str())
            .collect();
        Ok(Turn {
            text,
            raw: reply["content"].clone(),
        })
    }
}

/// OpenAI chat completions (ChatGPT, and most local servers).
fn openai(
    base: String,
    key: Option<String>,
    model: String,
) -> impl FnMut(&[Value]) -> Result<Turn> {
    let mut post = poster();
    let url = format!("{}/chat/completions", base.trim_end_matches('/'));
    move |messages: &[Value]| {
        let mut all = vec![json!({"role": "system", "content": system()})];
        all.extend(messages.iter().cloned());
        let headers: Vec<_> = key
            .iter()
            .map(|k| ("authorization", format!("Bearer {k}")))
            .collect();
        let reply = post(&url, &headers, &json!({"model": model, "messages": all}))?;
        let choice = &reply["choices"][0];
        if let Some(r) = choice["message"]["refusal"].as_str() {
            bail!("the model declined: {r}");
        }
        if choice["finish_reason"] == "length" {
            bail!(
                "the reply was cut off (the scene was too long, or the model's context is too small)"
            );
        }
        let text = choice["message"]["content"]
            .as_str()
            .filter(|t| !t.trim().is_empty())
            .context("the model sent an empty reply")?
            .to_string();
        Ok(Turn {
            raw: Value::String(text.clone()),
            text,
        })
    }
}

/// The Claude Code CLI in print mode, with no tools and no saved session, run from the temp
/// folder (so no project's CLAUDE.md is read). It uses whatever login the CLI has.
fn cli(model: String) -> impl FnMut(&[Value]) -> Result<Turn> {
    move |messages: &[Value]| {
        use std::io::{Read, Write};
        let dir = std::env::temp_dir().join("cornell-rtx-describe");
        std::fs::create_dir_all(&dir)?;
        // One file per call: another instance (or a `--describe` run) writing its own while
        // this CLI reads would hand it a truncated prompt.
        static CALLS: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
        let n = CALLS.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let system_file = dir.join(format!("system-{}-{n}.txt", std::process::id()));
        std::fs::write(&system_file, system())?;
        let spawn = |program: &str, prefix: &[&str]| {
            let mut cmd = std::process::Command::new(program);
            cmd.args(prefix)
                .current_dir(&dir)
                .args([
                    "-p",
                    "--output-format",
                    "json",
                    "--tools",
                    "",
                    "--no-session-persistence",
                ])
                .arg("--system-prompt-file")
                .arg(&system_file)
                .stdin(std::process::Stdio::piped())
                .stdout(std::process::Stdio::piped())
                .stderr(std::process::Stdio::piped());
            if !model.is_empty() {
                cmd.args(["--model", &model]);
            }
            #[cfg(windows)]
            {
                use std::os::windows::process::CommandExt;
                cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW: no console flashes up
            }
            cmd.spawn()
        };
        // The native installer gives claude.exe; an npm install gives claude.cmd, which only
        // a shell runs.
        let child = match spawn("claude", &[]) {
            Err(e) if e.kind() == std::io::ErrorKind::NotFound && cfg!(windows) => {
                spawn("cmd", &["/d", "/c", "claude"])
            }
            r => r,
        };
        let mut child = child.map_err(|e| {
            anyhow::anyhow!("could not start the Claude Code CLI (`claude` on PATH): {e}")
        })?;
        // A write error means the CLI already exited: its stderr below says why.
        let _ = child
            .stdin
            .take()
            .unwrap()
            .write_all(cli_prompt(messages).as_bytes());
        let reader = |mut pipe: Box<dyn Read + Send>| {
            std::thread::spawn(move || {
                let mut v = Vec::new();
                let _ = pipe.read_to_end(&mut v);
                v
            })
        };
        let stdout = reader(Box::new(child.stdout.take().unwrap()));
        let stderr = reader(Box::new(child.stderr.take().unwrap()));
        // The same limit as the HTTP backends: a CLI waiting on a login prompt or a stalled
        // network must not leave Generate disabled until a restart.
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(900);
        let status = loop {
            if let Some(s) = child.try_wait()? {
                break s;
            }
            if std::time::Instant::now() > deadline {
                let _ = child.kill();
                let _ = child.wait();
                let _ = std::fs::remove_file(&system_file);
                bail!("the Claude Code CLI did not answer within 15 minutes");
            }
            std::thread::sleep(std::time::Duration::from_millis(200));
        };
        let _ = std::fs::remove_file(&system_file);
        let (stdout, stderr) = (
            stdout.join().unwrap_or_default(),
            stderr.join().unwrap_or_default(),
        );
        let reply: Value = serde_json::from_slice(&stdout).map_err(|_| {
            anyhow::anyhow!(
                "the Claude Code CLI failed: {}",
                String::from_utf8_lossy(&stderr).trim()
            )
        })?;
        let text = reply["result"].as_str().unwrap_or_default().to_string();
        if reply["is_error"] == true || !status.success() {
            bail!("the Claude Code CLI failed: {text}");
        }
        if text.trim().is_empty() {
            bail!("the Claude Code CLI sent an empty reply");
        }
        Ok(Turn {
            raw: Value::String(text.clone()),
            text,
        })
    }
}

/// The conversation as one prompt (each CLI call is a fresh session).
fn cli_prompt(messages: &[Value]) -> String {
    if let [only] = messages {
        return only["content"].as_str().unwrap_or_default().to_string();
    }
    messages
        .iter()
        .map(|m| {
            let text = m["content"].as_str().unwrap_or_default();
            if m["role"] == "assistant" {
                format!("<your_previous_reply>\n{text}\n</your_previous_reply>")
            } else {
                text.to_string()
            }
        })
        .collect::<Vec<_>>()
        .join("\n\n")
}

/// The scene JSON in a reply: the text between the first `{` and the last `}` (the model is
/// asked for bare JSON, but a code fence or a sentence around it is tolerated).
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
        let turn = send(&messages)?;
        let result = extract(&turn.text)
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
                messages.push(json!({"role": "assistant", "content": turn.raw}));
                messages.push(json!({"role": "user", "content": format!(
                    "That scene was rejected: {last}. Reply with the corrected scene file as JSON only."
                )}));
            }
        }
    }
    bail!("the scene did not validate: {last}")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn turn(text: &str) -> Turn {
        Turn {
            text: text.into(),
            raw: json!([{"type": "thinking", "thinking": "", "signature": "x"},
                        {"type": "text", "text": text}]),
        }
    }

    #[test]
    fn an_invalid_scene_is_sent_back_and_the_fix_accepted() {
        let mut calls = vec![];
        let mut answers = vec![
            turn(
                r#"```json
{"format": 1, "title": "Bad", "settings": {"spp": 1000}}
```"#,
            ),
            turn(r#"{"format": 1, "title": "Two boxes", "settings": {"spp": 4}}"#),
        ]
        .into_iter();
        let mut send = |m: &[Value]| {
            calls.push(m.to_vec());
            Ok(answers.next().unwrap())
        };
        let s = generate("two boxes", &mut send).unwrap();
        assert_eq!(s.scene_name, "Two boxes");
        assert_eq!(s.spp, 4);
        assert_eq!(calls.len(), 2);
        let second = &calls[1];
        assert_eq!(second.len(), 3, "prompt, rejected reply, the error");
        assert_eq!(
            second[1]["content"][0]["type"], "thinking",
            "sent back unchanged"
        );
        assert!(second[2]["content"].as_str().unwrap().contains("rejected"));
    }

    #[test]
    fn hopeless_replies_are_errors() {
        let mut n = 0;
        let mut send = |_: &[Value]| {
            n += 1;
            Ok(turn("no json here"))
        };
        assert!(generate("x", &mut send).is_err());
        assert_eq!(n, REPAIRS + 1);
    }

    #[test]
    fn the_cli_sees_the_whole_conversation() {
        let m = [
            json!({"role": "user", "content": "a lamp"}),
            json!({"role": "assistant", "content": "{bad}"}),
            json!({"role": "user", "content": "That scene was rejected"}),
        ];
        let p = cli_prompt(&m);
        assert!(
            p.starts_with("a lamp")
                && p.contains("<your_previous_reply>\n{bad}")
                && p.ends_with("rejected")
        );
        assert_eq!(cli_prompt(&m[..1]), "a lamp");
    }

    #[test]
    fn missing_settings_are_reported() {
        let mut c = AiConfig::default();
        assert!(c.missing("").is_none(), "the CLI needs nothing");
        c.provider = Provider::Custom;
        assert!(c.missing("").is_some(), "a custom endpoint needs a model");
        c.model = "llama3.3".into();
        assert!(c.missing("").is_none(), "local servers need no key");
        c.provider = Provider::OpenAi;
        assert!(c.missing("sk-x").is_none());
    }
}
