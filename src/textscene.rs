//! Track 19 text-to-scene: a description goes to a language model with the `Settings` source
//! as its schema, and the JSON that comes back goes through `validate()`. A rejected scene is
//! sent back with the error for another try.
//!
//! Three tasks (`Task`): make a new scene, edit the current one ("make it night"), or critique
//! the current render (the image and the scene go to the model, which answers with notes on
//! lighting and composition and an improved scene).
//!
//! Backends (`Provider`, chosen in Menu > Text-to-scene model; model and reasoning under the
//! text box; all kept in prefs.json):
//! - the Claude Code CLI (`claude -p`), which uses the CLI's own login (a Claude
//!   subscription works, no API key);
//! - the Claude API (Messages API over raw HTTP; Rust has no official SDK), key from
//!   `ANTHROPIC_API_KEY` or the menu;
//! - the Codex CLI (`codex exec`), OpenAI's CLI with its own login (a ChatGPT account works);
//! - the opencode CLI (`opencode run`), with the providers set up in opencode (models picked
//!   from `opencode models`);
//! - the OpenAI API, key from `OPENAI_API_KEY` or the menu;
//! - a custom endpoint, OpenAI- or Anthropic-compatible (Ollama, LM Studio, llama.cpp,
//!   vLLM and similar local servers), key optional, with its model list fetched.
//!
//! Keys typed in the menu live for the session only; prefs.json never holds them.
use crate::scene::Settings;
use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

const CLAUDE_MODEL: &str = "claude-opus-5-5";
const ANTHROPIC_BASE: &str = "https://api.anthropic.com/v1";
const OPENAI_BASE: &str = "https://api.openai.com/v1";
const ANTHROPIC_VERSION: &str = "2026-10-21";
/// Tries after the first when the scene does not validate.
const REPAIRS: usize = 2;
/// Retries of one request on 429, 5xx (529 overloaded included) and connection errors.
const RETRIES: u32 = 3;
/// The longest a request or CLI run may take.
const TIMEOUT: std::time::Duration = std::time::Duration::from_secs(900);
/// The render sent for a critique is scaled to fit this (vision models downscale anyway).
pub const CRITIQUE_SIZE: u32 = 1568;

#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum Provider {
    #[default]
    ClaudeCli,
    ClaudeApi,
    CodexCli,
    OpenAi,
    Custom,
    OpencodeCli,
}
impl Provider {
    pub const ALL: [Provider; 6] = [
        Self::ClaudeCli,
        Self::ClaudeApi,
        Self::CodexCli,
        Self::OpencodeCli,
        Self::OpenAi,
        Self::Custom,
    ];
    pub fn label(self) -> &'static str {
        match self {
            Self::ClaudeCli => "Claude Code CLI (your Claude login)",
            Self::ClaudeApi => "Claude API (API key)",
            Self::CodexCli => "ChatGPT via Codex CLI (your ChatGPT login)",
            Self::OpenAi => "OpenAI API (API key)",
            Self::Custom => "Custom endpoint (local models)",
            Self::OpencodeCli => "opencode CLI (its providers and logins)",
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
    /// Reasoning levels the backend takes ("" = the model's default).
    pub fn efforts(self, protocol: Protocol) -> &'static [&'static str] {
        let claude: &'static [&'static str] = &["", "low", "medium", "high", "xhigh", "max"];
        let openai: &'static [&'static str] = &["", "minimal", "low", "medium", "high"];
        match self {
            Self::ClaudeCli | Self::ClaudeApi => claude,
            Self::CodexCli | Self::OpenAi => openai,
            // Model variants (`provider/model#variant`) differ per model: typed into the name.
            Self::OpencodeCli => &[""],
            Self::Custom => match protocol {
                Protocol::OpenAi => openai,
                Protocol::Anthropic => claude,
            },
        }
    }
    /// Model names offered before (or without) a fetch.
    pub fn suggested_models(self) -> &'static [&'static str] {
        match self {
            // The CLI's aliases for the latest of each family.
            Self::ClaudeCli => &["opus", "sonnet", "haiku", "fable"],
            Self::ClaudeApi => &[
                "claude-opus-5-5",
                "claude-sonnet-5-5",
                "claude-haiku-5-5",
                "claude-fable-5-1",
            ],
            _ => &[],
        }
    }
    /// The model list can be fetched from the server.
    pub fn can_fetch(self) -> bool {
        matches!(
            self,
            Self::ClaudeApi | Self::OpenAi | Self::Custom | Self::OpencodeCli
        )
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
    /// Empty: the provider's default (the CLIs' own choice, `claude-opus-5-5` on the Claude
    /// API). The OpenAI API and custom endpoints need one.
    pub model: String,
    /// Reasoning effort, one of `Provider::efforts` ("" = the model's default).
    pub effort: String,
    /// Custom endpoint base URL, e.g. `http://localhost:11434/v1` (Ollama).
    pub base_url: String,
    pub protocol: Protocol,
}
impl Default for AiConfig {
    fn default() -> Self {
        Self {
            provider: Provider::ClaudeCli,
            model: String::new(),
            effort: String::new(),
            base_url: "http://localhost:11434/v1".into(),
            protocol: Protocol::OpenAi,
        }
    }
}
impl AiConfig {
    /// A short name for the backend picker: the provider, plus the model or endpoint.
    pub fn short_label(&self) -> String {
        let provider = self.provider.label().split(" (").next().unwrap_or_default();
        let detail = if self.provider == Provider::Custom {
            format!("{} {}", self.base_url, self.model)
        } else {
            self.model.clone()
        };
        match detail.trim() {
            "" => provider.to_string(),
            d => format!("{provider} · {d}"),
        }
    }
    /// Why a request cannot be sent yet (shown under the Generate button), given the key
    /// typed in the menu.
    pub fn missing(&self, key: &str) -> Option<String> {
        if let Some(env) = self.provider.key_env()
            && key.trim().is_empty()
            && env_key(env).is_none()
        {
            return Some(format!(
                "Set {env} or type a key in Menu > Text-to-scene model"
            ));
        }
        if self.provider == Provider::Custom && self.base_url.trim().is_empty() {
            return Some("Set the endpoint URL in Menu > Text-to-scene model".into());
        }
        (matches!(self.provider, Provider::OpenAi | Provider::Custom)
            && self.model.trim().is_empty())
        .then(|| "Choose a model below (Fetch lists the server's models)".into())
    }
    /// A short name for messages ("Claude made …").
    pub fn who(&self) -> String {
        match self.provider {
            Provider::ClaudeCli | Provider::ClaudeApi => "Claude".into(),
            Provider::CodexCli => "ChatGPT".into(),
            Provider::OpenAi | Provider::Custom => self.model.trim().to_string(),
            Provider::OpencodeCli => match self.model.trim() {
                "" => "opencode".into(),
                m => m.rsplit('/').next().unwrap_or(m).to_string(),
            },
        }
    }
    fn effort(&self) -> Option<&str> {
        let e = self.effort.trim();
        (!e.is_empty() && self.provider.efforts(self.protocol).contains(&e)).then_some(e)
    }
    fn key(&self, typed: &str) -> Option<String> {
        Some(typed.trim().to_string())
            .filter(|k| !k.is_empty())
            .or_else(|| self.provider.key_env().and_then(env_key))
    }
    fn base(&self) -> String {
        match self.provider {
            Provider::OpenAi => OPENAI_BASE.into(),
            Provider::ClaudeApi => ANTHROPIC_BASE.into(),
            _ => self.base_url.trim().trim_end_matches('/').to_string(),
        }
    }
    /// `{base}/{path}` for the Anthropic protocol, adding `/v1` when the base lacks it.
    fn anthropic_url(&self, path: &str) -> String {
        let base = self.base();
        if base.ends_with("/v1") {
            format!("{base}/{path}")
        } else {
            format!("{base}/v1/{path}")
        }
    }
    fn uses_anthropic(&self) -> bool {
        self.provider == Provider::ClaudeApi
            || (self.provider == Provider::Custom && self.protocol == Protocol::Anthropic)
    }
}

fn env_key(name: &str) -> Option<String> {
    std::env::var(name).ok().filter(|k| !k.trim().is_empty())
}

fn system() -> String {
    let materials: String = crate::presets::MATERIAL_NAMES
        .iter()
        .enumerate()
        .map(|(i, name)| {
            format!(
                "{name}: {}\n",
                serde_json::to_string(&crate::presets::material(i)).unwrap_or_default()
            )
        })
        .collect();
    let generators: String = crate::generators::FAMILIES
        .iter()
        .map(|(id, name, what)| format!("{id} ({name}): {what}\n"))
        .collect();
    format!(
        "You design scenes for Caustica, a path tracer. Reply with one scene file as JSON: \
         {{\"format\": 1, \"title\": ..., \"description\": ..., \"tags\": [...], \"settings\": {{...}}}}. \
         `settings` is the serde form of the Rust `Settings` struct below; omitted fields take their defaults, \
         so set only what the scene needs. Every value must pass `Settings::validate()` (the ranges are in its \
         source). Units: the Cornell room spans -1..1; the floor is at y = -1; an object's world size is \
         size × 0.35 (× stretch on y, × scale_xz on x and z). Keep it under 200 objects. Unless asked for \
         notes, reply with the JSON and nothing else.\n\n\
         <material_library>\nNamed materials, as `Material` JSON; use these values when a scene wants one:\n{materials}</material_library>\n\n\
         <generators>\nProcedural families. A new scene may instead be \
         {{\"format\": 1, \"title\": ..., \"generator\": {{\"family\": id, \"seed\": number}}}}, which builds that \
         family (no other settings apply); each seed gives a different variant:\n{generators}</generators>\n\n\
         <scene_rs>\n{}\n</scene_rs>\n\n<example>\n{}\n</example>\n\n<example>\n{}\n</example>",
        include_str!("scene.rs"),
        include_str!("../scenes/builtin/desert-monolith.json"),
        include_str!("../scenes/builtin/cornell-caustic.json"),
    )
}

/// What the model is asked to do.
pub enum Task {
    /// A new scene from the description.
    New,
    /// The current scene, changed as described.
    Edit(Settings),
    /// Notes on the current render (a PNG) and an improved scene; the text is an optional focus.
    Critique { settings: Settings, png: Vec<u8> },
}

/// One message of the conversation. Assistant messages carry the backend's own content
/// (`raw`: Anthropic's blocks, thinking included) to send back unchanged.
#[derive(Clone)]
pub struct Msg {
    pub assistant: bool,
    pub text: String,
    pub png: Option<Arc<Vec<u8>>>,
    pub raw: Option<Value>,
}
impl Msg {
    fn user(text: String) -> Self {
        Self {
            assistant: false,
            text,
            png: None,
            raw: None,
        }
    }
}

/// One reply: its text, and the content to send back on a repair.
pub struct Turn {
    pub text: String,
    pub raw: Value,
}

/// Sends the conversation so far (the system prompt is added by the backend) and returns the
/// reply. Swapped out in tests.
pub type Transport<'a> = dyn FnMut(&[Msg]) -> Result<Turn> + 'a;
/// A ready backend (`transport`).
pub type Backend = Box<dyn FnMut(&[Msg]) -> Result<Turn> + Send>;

/// The result: the scene, and the model's notes (the critique, or anything it wrote around
/// the JSON).
pub struct Outcome {
    pub settings: Settings,
    pub notes: String,
}

/// The backend for `config`; `key` is the one typed in the menu (empty: the environment).
/// `cancel` stops a running CLI.
pub fn transport(config: &AiConfig, key: &str, cancel: Arc<AtomicBool>) -> Result<Backend> {
    if let Some(why) = config.missing(key) {
        bail!("{why}");
    }
    let config = config.clone();
    let key = config.key(key);
    Ok(match config.provider {
        Provider::ClaudeCli => Box::new(move |m: &[Msg]| claude_cli(&config, m, &cancel)),
        Provider::CodexCli => Box::new(move |m: &[Msg]| codex_cli(&config, m, &cancel)),
        Provider::OpencodeCli => Box::new(move |m: &[Msg]| opencode_cli(&config, m, &cancel)),
        _ if config.uses_anthropic() => {
            let mut post = poster();
            Box::new(move |m: &[Msg]| anthropic(&config, key.as_deref(), &mut *post, m))
        }
        _ => {
            let mut post = poster();
            Box::new(move |m: &[Msg]| openai(&config, key.as_deref(), &mut *post, m))
        }
    })
}

fn agent() -> Result<ureq::Agent> {
    // native-tls like the asset downloader, but with a long read timeout: a scene can take a
    // model a few minutes (a local one longer).
    let tls = native_tls::TlsConnector::new().map_err(|e| anyhow::anyhow!("TLS: {e}"))?;
    Ok(ureq::AgentBuilder::new()
        .tls_connector(Arc::new(tls))
        .timeout_connect(std::time::Duration::from_secs(15))
        .timeout(TIMEOUT)
        .build())
}

type Post = dyn FnMut(&str, &[(&str, String)], &Value) -> Result<Value> + Send;

/// Posts JSON with retries and backoff on transient failures.
fn poster() -> Box<Post> {
    let agent = agent();
    Box::new(move |url: &str, headers: &[(&str, String)], body: &Value| {
        let agent = agent.as_ref().map_err(|e| anyhow::anyhow!("{e}"))?;
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
    })
}

/// The auth headers of an Anthropic-protocol request.
fn anthropic_headers(config: &AiConfig, key: Option<&str>) -> Vec<(&'static str, String)> {
    let mut h = vec![("anthropic-version", ANTHROPIC_VERSION.to_string())];
    if let Some(k) = key {
        h.push(("x-api-key", k.to_string()));
        if config.provider == Provider::Custom {
            // Compatible proxies differ: some read the bearer header instead.
            h.push(("authorization", format!("Bearer {k}")));
        }
    }
    h
}

/// The Messages API. On api.anthropic.com: prompt caching and refusal fallbacks; compatible
/// servers get the plain fields (plus effort when one is chosen).
fn anthropic(
    config: &AiConfig,
    key: Option<&str>,
    post: &mut Post,
    messages: &[Msg],
) -> Result<Turn> {
    let official = config.provider == Provider::ClaudeApi;
    let mut headers = anthropic_headers(config, key);
    let model = match config.model.trim() {
        "" => CLAUDE_MODEL,
        m => m,
    };
    let wire: Vec<Value> = messages
        .iter()
        .map(|m| {
            if m.assistant {
                let content = m.raw.clone().unwrap_or_else(|| json!(m.text));
                return json!({"role": "assistant", "content": content});
            }
            let mut content = vec![];
            if let Some(png) = &m.png {
                content.push(json!({"type": "image", "source": {
                    "type": "base64", "media_type": "image/png", "data": base64(png)}}));
            }
            content.push(json!({"type": "text", "text": m.text}));
            json!({"role": "user", "content": content})
        })
        .collect();
    let mut body = json!({"model": model, "max_tokens": 32000, "messages": wire});
    if official {
        headers.push(("anthropic-beta", "server-side-fallback-2026-07-01".into()));
        body["fallbacks"] = json!("default");
        // The system prompt is the same on every call: retries and later prompts read it
        // from the cache.
        body["system"] =
            json!([{"type": "text", "text": system(), "cache_control": {"type": "ephemeral"}}]);
    } else {
        body["system"] = json!(system());
    }
    if let Some(e) = config.effort() {
        body["output_config"] = json!({"effort": e});
    }
    let reply = post(&config.anthropic_url("messages"), &headers, &body)?;
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

/// OpenAI chat completions (the OpenAI API, and most local servers).
fn openai(config: &AiConfig, key: Option<&str>, post: &mut Post, messages: &[Msg]) -> Result<Turn> {
    let mut wire = vec![json!({"role": "system", "content": system()})];
    for m in messages {
        wire.push(if m.assistant {
            json!({"role": "assistant", "content": m.text})
        } else if let Some(png) = &m.png {
            json!({"role": "user", "content": [
                {"type": "text", "text": m.text},
                {"type": "image_url", "image_url": {"url": format!("data:image/png;base64,{}", base64(png))}}
            ]})
        } else {
            json!({"role": "user", "content": m.text})
        });
    }
    let headers: Vec<_> = key
        .iter()
        .map(|k| ("authorization", format!("Bearer {k}")))
        .collect();
    let mut body = json!({"model": config.model.trim(), "messages": wire});
    if let Some(e) = config.effort() {
        body["reasoning_effort"] = json!(e);
    }
    let url = format!("{}/chat/completions", config.base());
    let reply = post(&url, &headers, &body)?;
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

/// The models a server offers (`GET /models`), sorted.
pub fn fetch_models(config: &AiConfig, key: &str) -> Result<Vec<String>> {
    if config.provider == Provider::OpencodeCli {
        return opencode_models();
    }
    let key = config.key(key);
    if let Some(env) = config.provider.key_env()
        && key.is_none()
    {
        bail!("Set {env} or type a key first");
    }
    let agent = agent()?;
    let (url, headers) = if config.uses_anthropic() {
        (
            config.anthropic_url("models?limit=1000"),
            anthropic_headers(config, key.as_deref()),
        )
    } else {
        let h: Vec<_> = key
            .iter()
            .map(|k| ("authorization", format!("Bearer {k}")))
            .collect();
        (format!("{}/models", config.base()), h)
    };
    let mut req = agent.get(&url);
    for (k, v) in &headers {
        req = req.set(k, v);
    }
    let reply: Value = match req.call() {
        Ok(r) => serde_json::from_reader(r.into_reader())?,
        Err(ureq::Error::Status(code, r)) => {
            bail!(
                "{url}: error {code}: {}",
                r.into_string().unwrap_or_default()
            )
        }
        Err(e) => bail!("{url}: {e}"),
    };
    let mut ids: Vec<String> = reply["data"]
        .as_array()
        .or_else(|| reply["models"].as_array())
        .context("no model list in the reply")?
        .iter()
        .filter_map(|m| m["id"].as_str().or_else(|| m["name"].as_str()))
        .map(str::to_string)
        .collect();
    ids.sort();
    ids.dedup();
    Ok(ids)
}

/// What a CLI run returned.
struct CliRun {
    status: std::process::ExitStatus,
    stdout: Vec<u8>,
    stderr: Vec<u8>,
}

/// Runs a CLI with `stdin` as its input, killing it on cancel or after `TIMEOUT`. On Windows
/// an npm install is `name.cmd`, which only a shell starts, so that is tried second.
fn run_cli(
    name: &str,
    args: &[String],
    dir: &std::path::Path,
    stdin: &str,
    cancel: &AtomicBool,
) -> Result<CliRun> {
    use std::io::{Read, Write};
    let spawn = |program: &str, prefix: &[&str]| {
        let mut cmd = std::process::Command::new(program);
        cmd.args(prefix)
            .args(args)
            .current_dir(dir)
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped());
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW: no console flashes up
        }
        cmd.spawn()
    };
    let child = match spawn(name, &[]) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound && cfg!(windows) => {
            spawn("cmd", &["/d", "/c", name])
        }
        r => r,
    };
    let mut child = child.map_err(|e| {
        anyhow::anyhow!("could not start `{name}` (is it installed and on PATH?): {e}")
    })?;
    // On its own thread, so a CLI that never reads (stuck at a login prompt) cannot block
    // Cancel. A write error means the CLI already exited: its stderr below says why.
    let mut input = child.stdin.take().unwrap();
    let text = stdin.to_string();
    std::thread::spawn(move || {
        let _ = input.write_all(text.as_bytes());
    });
    let reader = |mut pipe: Box<dyn Read + Send>| {
        std::thread::spawn(move || {
            let mut v = Vec::new();
            let _ = pipe.read_to_end(&mut v);
            v
        })
    };
    let out = reader(Box::new(child.stdout.take().unwrap()));
    let err = reader(Box::new(child.stderr.take().unwrap()));
    let deadline = std::time::Instant::now() + TIMEOUT;
    let status = loop {
        if let Some(s) = child.try_wait()? {
            break s;
        }
        let late = std::time::Instant::now() > deadline;
        if late || cancel.load(Ordering::Relaxed) {
            // The whole tree: an npm install runs node under `cmd /c`, and killing only cmd
            // would leave the model running and the call folder in use.
            #[cfg(windows)]
            {
                use std::os::windows::process::CommandExt;
                let _ = std::process::Command::new("taskkill")
                    .args(["/T", "/F", "/PID", &child.id().to_string()])
                    .creation_flags(0x0800_0000)
                    .output();
            }
            let _ = child.kill();
            let _ = child.wait();
            if late {
                bail!("`{name}` did not answer within 15 minutes");
            }
            bail!("cancelled");
        }
        std::thread::sleep(std::time::Duration::from_millis(200));
    };
    Ok(CliRun {
        status,
        stdout: out.join().unwrap_or_default(),
        stderr: err.join().unwrap_or_default(),
    })
}

/// A private folder for one CLI call (the prompt file and the render), removed afterwards.
struct CallDir(std::path::PathBuf);
impl CallDir {
    fn new() -> Result<Self> {
        static CALLS: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
        let n = CALLS.fetch_add(1, Ordering::Relaxed);
        let dir = std::env::temp_dir()
            .join("caustica-describe")
            .join(format!("{}-{n}", std::process::id()));
        std::fs::create_dir_all(&dir)?;
        Ok(Self(dir))
    }
    /// Writes the first message's render (if any); its file name.
    fn render(&self, messages: &[Msg]) -> Result<Option<String>> {
        match messages.first().and_then(|m| m.png.as_ref()) {
            Some(png) => {
                std::fs::write(self.0.join("render.png"), png.as_slice())?;
                Ok(Some("render.png".into()))
            }
            None => Ok(None),
        }
    }
}
impl Drop for CallDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// The Claude Code CLI in print mode with no saved session, run from a private temp folder
/// (so no project's CLAUDE.md is read). Tools are off, except Read for a critique's render.
fn claude_cli(config: &AiConfig, messages: &[Msg], cancel: &AtomicBool) -> Result<Turn> {
    let dir = CallDir::new()?;
    let system_file = dir.0.join("system.txt");
    std::fs::write(&system_file, system())?;
    let image = dir.render(messages)?;
    let mut args: Vec<String> = ["-p", "--output-format", "json", "--no-session-persistence"]
        .map(String::from)
        .into();
    if image.is_some() {
        args.extend(["--tools", "Read", "--allowedTools", "Read"].map(String::from));
    } else {
        args.extend(["--tools", ""].map(String::from));
    }
    args.push("--system-prompt-file".into());
    args.push(system_file.to_string_lossy().into_owned());
    if !config.model.trim().is_empty() {
        args.extend(["--model".into(), config.model.trim().to_string()]);
    }
    if let Some(e) = config.effort() {
        args.extend(["--effort".into(), e.to_string()]);
    }
    let mut prompt = cli_prompt(messages);
    if let Some(name) = &image {
        prompt = format!(
            "The render is the image file `{name}` in the current folder: read it first.\n\n{prompt}"
        );
    }
    let run = run_cli("claude", &args, &dir.0, &prompt, cancel)?;
    let reply: Value = serde_json::from_slice(&run.stdout).map_err(|_| {
        anyhow::anyhow!(
            "the Claude Code CLI failed: {}",
            String::from_utf8_lossy(&run.stderr).trim()
        )
    })?;
    let text = reply["result"].as_str().unwrap_or_default().to_string();
    if reply["is_error"] == true || !run.status.success() {
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

/// OpenAI's Codex CLI, non-interactive (`codex exec`), read-only sandbox, run from a private
/// temp folder; it uses the CLI's own login (`codex login`: a ChatGPT account or an API key).
/// It has no system prompt option, so the instructions lead the prompt.
fn codex_cli(config: &AiConfig, messages: &[Msg], cancel: &AtomicBool) -> Result<Turn> {
    let dir = CallDir::new()?;
    let last = dir.0.join("reply.txt");
    let image = dir.render(messages)?;
    let mut args: Vec<String> = [
        "exec",
        "--skip-git-repo-check",
        "--sandbox",
        "read-only",
        "--color",
        "never",
    ]
    .map(String::from)
    .into();
    args.push("--output-last-message".into());
    args.push(last.to_string_lossy().into_owned());
    if !config.model.trim().is_empty() {
        args.extend(["--model".into(), config.model.trim().to_string()]);
    }
    if let Some(e) = config.effort() {
        args.extend(["-c".into(), format!("model_reasoning_effort=\"{e}\"")]);
    }
    args.push("-".into()); // the prompt comes on stdin
    // After the prompt: `--image` takes several files and would read `-` as one.
    if let Some(name) = &image {
        args.extend([
            "--image".into(),
            dir.0.join(name).to_string_lossy().into_owned(),
        ]);
    }
    let prompt = format!(
        "<instructions>\n{}\n</instructions>\n\nDo not run commands or edit files: answer in your reply only.\n\n{}",
        system(),
        cli_prompt(messages)
    );
    let run = run_cli("codex", &args, &dir.0, &prompt, cancel)?;
    let text = std::fs::read_to_string(&last).unwrap_or_default();
    if !run.status.success() || text.trim().is_empty() {
        let err = String::from_utf8_lossy(&run.stderr);
        let mut tail: Vec<&str> = err.lines().rev().take(6).collect();
        tail.reverse();
        bail!("the Codex CLI failed: {}", tail.join("\n"));
    }
    Ok(Turn {
        raw: Value::String(text.clone()),
        text,
    })
}

/// The opencode CLI (`opencode run`), with the providers and logins set up in opencode
/// (`opencode auth login`; models as `provider/model`, listed by `opencode models`). Run from a
/// private temp folder without `--auto`, so it gets no permission to edit or run anything.
/// The reply is its JSON event stream: the `text` parts of the last message, or an `error`.
fn opencode_cli(config: &AiConfig, messages: &[Msg], cancel: &AtomicBool) -> Result<Turn> {
    let dir = CallDir::new()?;
    let image = dir.render(messages)?;
    let mut args: Vec<String> = ["run", "--format", "json"].map(String::from).into();
    if !config.model.trim().is_empty() {
        args.extend(["--model".into(), config.model.trim().to_string()]);
    }
    if let Some(name) = &image {
        args.extend([
            "--file".into(),
            dir.0.join(name).to_string_lossy().into_owned(),
        ]);
    }
    let prompt = format!(
        "<instructions>\n{}\n</instructions>\n\nDo not use tools, run commands or edit files: answer in your reply only.\n\n{}",
        system(),
        cli_prompt(messages)
    );
    let run = run_cli("opencode", &args, &dir.0, &prompt, cancel)?;
    let text = opencode_reply(&String::from_utf8_lossy(&run.stdout)).map_err(|e| {
        let err = String::from_utf8_lossy(&run.stderr);
        let mut tail: Vec<&str> = err.lines().rev().take(4).collect();
        tail.reverse();
        anyhow::anyhow!(
            "the opencode CLI failed: {e}{}",
            if tail.is_empty() {
                String::new()
            } else {
                format!("\n{}", tail.join("\n"))
            }
        )
    })?;
    Ok(Turn {
        raw: Value::String(text.clone()),
        text,
    })
}
/// The reply in `opencode run --format json`'s event lines: the text parts of the last message
/// that has any, joined; an `error` event is the failure.
fn opencode_reply(events: &str) -> Result<String> {
    let mut message = String::new();
    let mut parts: Vec<String> = vec![];
    for line in events.lines() {
        let Ok(e) = serde_json::from_str::<Value>(line) else {
            continue;
        };
        match e["type"].as_str() {
            Some("error") => {
                let m = e["error"]["message"]
                    .as_str()
                    .or(e["error"]["data"]["message"].as_str());
                bail!("{}", m.unwrap_or("unknown error"));
            }
            Some("text") => {
                let id = e["part"]["messageID"].as_str().unwrap_or_default();
                if id != message {
                    message = id.to_string();
                    parts.clear();
                }
                parts.push(e["part"]["text"].as_str().unwrap_or_default().to_string());
            }
            _ => {}
        }
    }
    let text = parts.concat();
    if text.trim().is_empty() {
        bail!("an empty reply");
    }
    Ok(text)
}
/// `opencode models`: one `provider/model` per line.
fn opencode_models() -> Result<Vec<String>> {
    let dir = CallDir::new()?;
    let run = run_cli(
        "opencode",
        &["models".into()],
        &dir.0,
        "",
        &AtomicBool::new(false),
    )?;
    let out = String::from_utf8_lossy(&run.stdout);
    let mut models: Vec<String> = out
        .lines()
        .map(str::trim)
        .filter(|l| l.contains('/') && !l.contains(' '))
        .map(String::from)
        .collect();
    if !run.status.success() || models.is_empty() {
        bail!(
            "`opencode models` listed nothing: {}",
            String::from_utf8_lossy(&run.stderr).trim()
        );
    }
    models.sort();
    models.dedup();
    Ok(models)
}

/// The conversation as one prompt (each CLI call is a fresh session).
fn cli_prompt(messages: &[Msg]) -> String {
    if let [only] = messages {
        return only.text.clone();
    }
    messages
        .iter()
        .map(|m| {
            if m.assistant {
                format!("<your_previous_reply>\n{}\n</your_previous_reply>", m.text)
            } else {
                m.text.clone()
            }
        })
        .collect::<Vec<_>>()
        .join("\n\n")
}

fn base64(bytes: &[u8]) -> String {
    const T: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut s = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for c in bytes.chunks(3) {
        let n = ((c[0] as u32) << 16)
            | ((*c.get(1).unwrap_or(&0) as u32) << 8)
            | *c.get(2).unwrap_or(&0) as u32;
        for i in 0..4 {
            if i <= c.len() {
                s.push(T[((n >> (18 - 6 * i)) & 63) as usize] as char);
            } else {
                s.push('=');
            }
        }
    }
    s
}

/// The scene file JSON of `s` (what the model edits).
fn scene_json(s: &Settings) -> String {
    serde_json::to_string(&json!({"format": 1, "title": s.scene_name, "settings": s}))
        .unwrap_or_default()
}

/// The JSON object in a reply and the text around it (the notes). The model is asked for
/// bare JSON, but a code fence or notes around it are tolerated: the object runs from a `{`
/// to the last `}`, starting at the first `{` where that span parses, so a brace in the notes
/// ("the {key} light") is skipped.
fn extract(text: &str) -> Option<(&str, String)> {
    let b = text.rfind('}')?;
    let notes = |a: usize| {
        let n = format!(
            "{}
{}",
            &text[..a],
            &text[b + 1..]
        );
        n.replace("```json", "")
            .replace("```", "")
            .trim()
            .to_string()
    };
    let starts: Vec<usize> = text[..b].match_indices('{').map(|(i, _)| i).collect();
    starts
        .iter()
        .find(|&&a| serde_json::from_str::<Value>(&text[a..=b]).is_ok())
        .or(starts.first())
        .map(|&a| (&text[a..=b], notes(a)))
}

/// The first message for `task`.
fn first_message(prompt: &str, task: &Task) -> Msg {
    let prompt = prompt.trim();
    match task {
        Task::New => Msg::user(prompt.to_string()),
        Task::Edit(s) => Msg::user(format!(
            "This is the current scene file:\n<scene>\n{}\n</scene>\n\nChange it as follows: {prompt}\n\n\
             Keep everything the request does not touch exactly as it is. Reply with the whole changed \
             scene file as JSON.",
            scene_json(s)
        )),
        Task::Critique { settings, png } => {
            let focus = if prompt.is_empty() {
                String::new()
            } else {
                format!("\n\nFocus on: {prompt}")
            };
            Msg {
                png: Some(Arc::new(png.clone())),
                ..Msg::user(format!(
                    "The attached image is the current render of this scene file:\n<scene>\n{}\n</scene>\n\n\
                     Critique it as a lighting artist and art director would: up to five concrete points on \
                     lighting, composition, colour and materials, each with the change that fixes it. Write the \
                     critique first as short plain-text lines, then the improved scene file as JSON (the whole \
                     file, every change applied, everything else kept).{focus}",
                    scene_json(settings)
                ))
            }
        }
    }
}

/// Runs `task`, repairing up to `REPAIRS` times. `progress` gets a status line before each
/// request; `cancel` stops between requests.
pub fn generate(
    prompt: &str,
    task: &Task,
    send: &mut Transport,
    progress: &mut dyn FnMut(String),
    cancel: &AtomicBool,
) -> Result<Outcome> {
    let mut messages = vec![first_message(prompt, task)];
    let mut last = String::new();
    // The first notes win: a repair is asked for JSON only, so it has none.
    let mut first_notes = String::new();
    for attempt in 0..=REPAIRS {
        if cancel.load(Ordering::Relaxed) {
            bail!("cancelled");
        }
        progress(if attempt == 0 {
            "Waiting for the model".into()
        } else {
            format!(
                "Fixing the scene (try {} of {}): {last}",
                attempt + 1,
                REPAIRS + 1
            )
        });
        let turn = send(&messages)?;
        let mut notes = String::new();
        let result = extract(&turn.text)
            .context("no JSON object in the reply")
            .and_then(|(j, n)| {
                if first_notes.is_empty() {
                    first_notes = n.clone();
                }
                notes = n;
                Ok(serde_json::from_str::<Value>(j)?)
            })
            .and_then(crate::scenes::parse)
            .and_then(|mut s| {
                s.validate()?;
                Ok(s)
            });
        match result {
            Ok(settings) => {
                if notes.is_empty() {
                    notes = first_notes;
                }
                return Ok(Outcome { settings, notes });
            }
            Err(e) => {
                last = format!("{e:#}");
                messages.push(Msg {
                    assistant: true,
                    text: turn.text,
                    png: None,
                    raw: Some(turn.raw),
                });
                messages.push(Msg::user(format!(
                    "That scene was rejected: {last}. Reply with the corrected scene file as JSON only."
                )));
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
    fn run(task: &Task, send: &mut Transport) -> Result<Outcome> {
        generate(
            "two boxes",
            task,
            send,
            &mut |_| {},
            &AtomicBool::new(false),
        )
    }

    #[test]
    fn an_invalid_scene_is_sent_back_and_the_fix_accepted() {
        let mut calls: Vec<Vec<Msg>> = vec![];
        let mut answers = vec![
            turn(
                r#"```json
{"format": 1, "title": "Bad", "settings": {"spp": 1000}}
```"#,
            ),
            turn(r#"{"format": 1, "title": "Two boxes", "settings": {"spp": 4}}"#),
        ]
        .into_iter();
        let mut send = |m: &[Msg]| {
            calls.push(m.to_vec());
            Ok(answers.next().unwrap())
        };
        let o = run(&Task::New, &mut send).unwrap();
        assert_eq!(o.settings.scene_name, "Two boxes");
        assert_eq!(o.settings.spp, 4);
        assert_eq!(calls.len(), 2);
        let second = &calls[1];
        assert_eq!(second.len(), 3, "prompt, rejected reply, the error");
        assert_eq!(
            second[1].raw.as_ref().unwrap()[0]["type"],
            "thinking",
            "sent back unchanged"
        );
        assert!(second[2].text.contains("rejected"));
    }

    #[test]
    fn braces_in_notes_and_notes_lost_in_a_repair() {
        let (json, notes) = extract(
            "Light the {key} side.
```json
{\"a\": {\"b\": 1}}
```",
        )
        .unwrap();
        assert_eq!(json, "{\"a\": {\"b\": 1}}");
        assert_eq!(notes, "Light the {key} side.");
        let mut answers = vec![
            turn(
                "1. Warmer key.
{\"format\": 1, \"title\": \"X\", \"settings\": {\"spp\": 1000}}",
            ),
            turn("{\"format\": 1, \"title\": \"X\", \"settings\": {\"spp\": 2}}"),
        ]
        .into_iter();
        let mut send = |_: &[Msg]| Ok(answers.next().unwrap());
        let o = run(&Task::New, &mut send).unwrap();
        assert_eq!(o.notes, "1. Warmer key.");
    }

    #[test]
    fn hopeless_replies_are_errors() {
        let mut n = 0;
        let mut send = |_: &[Msg]| {
            n += 1;
            Ok(turn("no json here"))
        };
        assert!(run(&Task::New, &mut send).is_err());
        assert_eq!(n, REPAIRS + 1);
    }

    #[test]
    fn edits_carry_the_scene_and_critiques_the_render_and_notes() {
        let current = Settings {
            scene_name: "Lamp room".into(),
            spp: 3,
            ..Settings::default()
        };
        let mut seen = vec![];
        let mut send = |m: &[Msg]| {
            seen.push(m[0].clone());
            Ok(turn(
                "1. The key light is flat: raise it.\n{\"format\": 1, \"title\": \"Lamp room\", \"settings\": {\"spp\": 2}}",
            ))
        };
        let o = run(&Task::Edit(current.clone()), &mut send).unwrap();
        let o2 = run(
            &Task::Critique {
                settings: current,
                png: vec![1, 2, 3],
            },
            &mut send,
        )
        .unwrap();
        assert!(seen[0].text.contains("\"Lamp room\"") && seen[0].text.contains("two boxes"));
        assert!(seen[0].png.is_none());
        assert_eq!(
            seen[1].png.as_deref().map(Vec::as_slice),
            Some(&[1u8, 2, 3][..])
        );
        assert_eq!(o2.notes, "1. The key light is flat: raise it.");
        assert_eq!((o.settings.spp, o2.settings.spp), (2, 2));
    }

    #[test]
    fn cancel_stops_before_the_next_request() {
        let mut send = |_: &[Msg]| -> Result<Turn> { panic!("no request after cancel") };
        let r = generate(
            "x",
            &Task::New,
            &mut send,
            &mut |_| {},
            &AtomicBool::new(true),
        );
        assert!(r.is_err());
    }

    #[test]
    fn the_cli_sees_the_whole_conversation() {
        let m = [
            Msg::user("a lamp".into()),
            Msg {
                assistant: true,
                ..Msg::user("{bad}".into())
            },
            Msg::user("That scene was rejected".into()),
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
    fn missing_settings_and_efforts_per_provider() {
        let mut c = AiConfig::default();
        assert!(c.missing("").is_none(), "the Claude CLI needs nothing");
        c.provider = Provider::CodexCli;
        assert!(c.missing("").is_none(), "nor does the Codex CLI");
        c.provider = Provider::Custom;
        assert!(c.missing("").is_some(), "a custom endpoint needs a model");
        c.model = "llama3.3".into();
        assert!(c.missing("").is_none(), "local servers need no key");
        c.provider = Provider::OpenAi;
        assert!(c.missing("sk-x").is_none());
        c.effort = "xhigh".into();
        assert_eq!(c.effort(), None, "not an OpenAI level");
        c.provider = Provider::ClaudeApi;
        assert_eq!(c.effort(), Some("xhigh"));
        assert_eq!(
            c.anthropic_url("models"),
            "https://api.anthropic.com/v1/models"
        );
        c.provider = Provider::Custom;
        c.base_url = "http://localhost:8080/".into();
        assert_eq!(
            c.anthropic_url("messages"),
            "http://localhost:8080/v1/messages"
        );
    }

    #[test]
    fn opencode_events_give_the_last_message_or_the_error() {
        let ev = |kind: &str, msg: &str, text: &str| {
            json!({"type": kind, "part": {"messageID": msg, "text": text}}).to_string()
        };
        let stream = [
            ev("step_start", "a", ""),
            ev("text", "a", "thinking aloud"),
            ev("text", "b", "{\"objects\""),
            ev("text", "b", ": []}"),
            "not json".into(),
        ]
        .join(
            "
",
        );
        assert_eq!(opencode_reply(&stream).unwrap(), "{\"objects\": []}");
        let error = json!({"type": "error", "error": {"message": "Free usage limit reached."}});
        let e = opencode_reply(&error.to_string()).unwrap_err().to_string();
        assert!(e.contains("Free usage"), "{e}");
        assert!(opencode_reply("").is_err());
        let mut c = AiConfig {
            provider: Provider::OpencodeCli,
            ..AiConfig::default()
        };
        assert!(c.missing("").is_none(), "opencode needs no key or model");
        c.model = "lmstudio/qwen/qwen3-30b".into();
        assert_eq!(c.who(), "qwen3-30b");
    }
    /// A real critique through the Claude Code CLI (its login): `CRITIQUE_PNG=render.png cargo
    /// test -- --ignored live_critique --nocapture`.
    #[test]
    #[ignore]
    fn live_critique_through_the_claude_cli() {
        let png = std::fs::read(std::env::var("CRITIQUE_PNG").unwrap()).unwrap();
        let cancel = Arc::new(AtomicBool::new(false));
        let config = AiConfig {
            effort: "low".into(),
            ..AiConfig::default()
        };
        let mut send = transport(&config, "", cancel.clone()).unwrap();
        let task = Task::Critique {
            settings: Settings::default(),
            png,
        };
        let o = generate("", &task, &mut *send, &mut |s| println!("{s}"), &cancel).unwrap();
        println!(
            "NOTES:
{}
OBJECTS: {}",
            o.notes,
            o.settings.objects.len()
        );
        assert!(!o.notes.is_empty());
    }

    /// A real scene through the opencode CLI: `OPENCODE_MODEL=provider/model cargo test --
    /// --ignored live_scene_through_opencode --nocapture`.
    #[test]
    #[ignore]
    fn live_scene_through_opencode() {
        let cancel = Arc::new(AtomicBool::new(false));
        let config = AiConfig {
            provider: Provider::OpencodeCli,
            model: std::env::var("OPENCODE_MODEL").unwrap_or_default(),
            ..AiConfig::default()
        };
        assert!(!opencode_models().unwrap().is_empty());
        let mut send = transport(&config, "", cancel.clone()).unwrap();
        let task = Task::New;
        let o = generate(
            "a red ball on a blue box",
            &task,
            &mut *send,
            &mut |s| println!("{s}"),
            &cancel,
        )
        .unwrap();
        println!("OBJECTS: {}", o.settings.objects.len());
        assert!(!o.settings.objects.is_empty());
    }

    /// Model listing from a running server: `FETCH_BASE=http://localhost:11434/v1 cargo test
    /// -- --ignored live_fetch --nocapture` (OpenAI format, then Anthropic format).
    #[test]
    #[ignore]
    fn live_fetch_models() {
        let mut c = AiConfig {
            provider: Provider::Custom,
            base_url: std::env::var("FETCH_BASE").unwrap(),
            ..AiConfig::default()
        };
        for protocol in [Protocol::OpenAi, Protocol::Anthropic] {
            c.protocol = protocol;
            let models = fetch_models(&c, "").unwrap();
            println!("{protocol:?}: {models:?}");
            assert!(!models.is_empty());
        }
    }

    #[test]
    fn base64_matches_the_standard() {
        assert_eq!(base64(b""), "");
        assert_eq!(base64(b"f"), "Zg==");
        assert_eq!(base64(b"fo"), "Zm8=");
        assert_eq!(base64(b"foobar"), "Zm9vYmFy");
    }
}
