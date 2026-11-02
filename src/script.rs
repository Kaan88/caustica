//! Track 19 scripting: `--script file.rhai` runs a Rhai script that drives the app, for batch
//! renders and scripted edits. The script runs on its own thread; each call is a request the
//! app handles between frames (`App::poll_script`), and the call blocks until it is done.
//!
//! Functions: `load(scene)` (a scene file path, stem or title), `set(key, json)` (a top-level
//! setting, or a `/` path such as `objects/0/rotation`), `frames(n)` (render n frames),
//! `reset()` (restart accumulation), `capture(path)` (PNG or EXR of the current image),
//! `samples()` (samples accumulated so far), `scenes()` (every scene title). `print` writes to
//! stdout. When the script ends, a headless run exits with `SCRIPT_OK` (or `SCRIPT_ERROR`).
use std::sync::mpsc::{Receiver, Sender, channel};

type Res<T> = Result<T, Box<rhai::EvalAltResult>>;

pub enum Request {
    Load(String),
    Set(String, String),
    Frames(u32),
    Reset,
    Capture(String),
    Samples,
    Scenes,
}

pub enum Reply {
    Done(Result<(), String>),
    Number(i64),
    List(Vec<String>),
}

pub struct Script {
    requests: Receiver<(Request, Sender<Reply>)>,
    /// The script thread's end: Ok, or the error it stopped with.
    finished: Receiver<Result<(), String>>,
    /// A `frames(n)` call in progress: frames still to render, and who to tell.
    pub waiting: Option<(u32, Sender<Reply>)>,
    /// A `load`, `set` or `reset` has not been rendered yet: `capture` and `samples` would
    /// report the previous image.
    pub stale: bool,
}

impl Script {
    pub fn start(path: std::path::PathBuf) -> anyhow::Result<Self> {
        let source = std::fs::read_to_string(&path)?;
        let (tx, requests) = channel::<(Request, Sender<Reply>)>();
        let (done, finished) = channel();
        std::thread::spawn(move || {
            let call = move |r: Request| -> Result<Reply, Box<rhai::EvalAltResult>> {
                let (reply_tx, reply_rx) = channel();
                tx.send((r, reply_tx)).map_err(|_| "the app has closed")?;
                reply_rx.recv().map_err(|_| "the app has closed".into())
            };
            let done_or = |r: Reply| match r {
                Reply::Done(Err(e)) => Err::<(), Box<rhai::EvalAltResult>>(e.into()),
                _ => Ok(()),
            };
            let mut engine = rhai::Engine::new();
            let c = call.clone();
            engine.register_fn("load", move |s: &str| done_or(c(Request::Load(s.into()))?));
            let c = call.clone();
            engine.register_fn("set", move |k: &str, v: &str| {
                done_or(c(Request::Set(k.into(), v.into()))?)
            });
            let c = call.clone();
            engine.register_fn("frames", move |n: i64| {
                done_or(c(Request::Frames(n.clamp(0, 1 << 24) as u32))?)
            });
            let c = call.clone();
            engine.register_fn("reset", move || done_or(c(Request::Reset)?));
            let c = call.clone();
            engine.register_fn("capture", move |p: &str| {
                done_or(c(Request::Capture(p.into()))?)
            });
            let c = call.clone();
            engine.register_fn("samples", move || -> Res<i64> {
                match c(Request::Samples)? {
                    Reply::Number(n) => Ok(n),
                    Reply::Done(Err(e)) => Err(e.into()),
                    _ => Ok(0),
                }
            });
            let c = call;
            engine.register_fn("scenes", move || -> Res<rhai::Array> {
                match c(Request::Scenes)? {
                    Reply::List(l) => Ok(l
                        .into_iter()
                        .map(rhai::Dynamic::from)
                        .collect::<rhai::Array>()),
                    _ => Ok(rhai::Array::new()),
                }
            });
            let result = engine.run(&source).map_err(|e| e.to_string());
            let _ = done.send(result);
        });
        Ok(Self {
            requests,
            finished,
            waiting: None,
            stale: false,
        })
    }
    /// The next request, unless a `frames` call is still running. Waits up to `wait` for one
    /// (headless runs wait, so no frame renders between two calls: the output is the same
    /// on every run).
    pub fn next(&mut self, wait: std::time::Duration) -> Option<(Request, Sender<Reply>)> {
        if self.waiting.is_some() {
            return None;
        }
        if wait.is_zero() {
            self.requests.try_recv().ok()
        } else {
            self.requests.recv_timeout(wait).ok()
        }
    }
    /// Counts one rendered frame towards a running `frames(n)`.
    pub fn frame_done(&mut self) {
        self.stale = false;
        if let Some((n, _)) = &mut self.waiting {
            *n = n.saturating_sub(1);
            if *n == 0
                && let Some((_, tx)) = self.waiting.take()
            {
                let _ = tx.send(Reply::Done(Ok(())));
            }
        }
    }
    /// The script's result once it has ended (a panicked script thread is an error).
    pub fn finished(&self) -> Option<Result<(), String>> {
        match self.finished.try_recv() {
            Ok(r) => Some(r),
            Err(std::sync::mpsc::TryRecvError::Empty) => None,
            Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                Some(Err("the script thread stopped (panic)".into()))
            }
        }
    }
}

/// Sets a top-level setting or a `/` path inside the settings to a JSON value, through the
/// same validation as a loaded file.
pub fn set(s: &mut crate::scene::Settings, key: &str, value: &str) -> anyhow::Result<()> {
    let mut j = serde_json::to_value(&*s)?;
    let pointer = format!("/{}", key.trim_start_matches('/'));
    let slot = j
        .pointer_mut(&pointer)
        .ok_or_else(|| anyhow::anyhow!("unknown setting {key}"))?;
    *slot =
        serde_json::from_str(value).or_else(|_| serde_json::from_str(&format!("\"{value}\"")))?;
    let mut next: crate::scene::Settings = serde_json::from_value(j)?;
    next.validate()?;
    *s = next;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn set_reaches_nested_fields_and_validates() {
        let mut s = crate::scene::Settings::default();
        set(&mut s, "objects/0/rotation", "33").unwrap();
        assert_eq!(s.objects[0].rotation, 33.);
        set(&mut s, "spp", "8").unwrap();
        assert_eq!(s.spp, 8);
        assert!(
            set(&mut s, "spp", "1000").is_err(),
            "validate() still applies"
        );
        assert_eq!(s.spp, 8, "a rejected value leaves the settings alone");
        assert!(set(&mut s, "no_such_key", "1").is_err());
    }
}
