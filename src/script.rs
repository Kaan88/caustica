//! Track 19 scripting: `--script file.rhai` runs a Rhai script that drives the app, for batch
//! renders and scripted edits. The script runs on its own thread; each call is a request the
//! app handles between frames (`App::poll_script`), and the call blocks until it is done.
//!
//! Functions: `load(scene)` (a scene file path, stem or title), `set(key, json)` (a top-level
//! setting, or a `/` path such as `objects/0/rotation`), `frames(n)` (render n frames),
//! `reset()` (restart accumulation), `capture(path)` (PNG or EXR of the current image),
//! `samples()` (samples accumulated so far), `get(key)` (a setting or `/` path as JSON text), `scenes()` (every scene title),
//! `add(json)` (an object from its JSON, as in a scene file: generators from scripts) and `clear()`
//! (every object removed). `print` writes to stdout and the console's log. When the script ends,
//! a headless run exits with `SCRIPT_OK` (or `SCRIPT_ERROR`).
//!
//! Wave 3: the in-app console (`console.rs`) runs the same API from a text box (`Script::run`,
//! stoppable), and `per_frame` runs an animation's per-frame script (`Animation::script`).
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::sync::{Arc, Mutex};

type Res<T> = Result<T, Box<rhai::EvalAltResult>>;

pub enum Request {
    Load(String),
    Set(String, String),
    Frames(u32),
    Reset,
    Capture(String),
    Samples,
    Scenes,
    Get(String),
    Add(String),
    Clear,
}

pub enum Reply {
    Done(Result<(), String>),
    Number(i64),
    List(Vec<String>),
    Text(String),
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
    /// `print` lines for the console.
    output: Receiver<String>,
    /// Set by `stop`: the engine ends the script at its next operation.
    stop: Arc<AtomicBool>,
}

impl Script {
    pub fn start(path: std::path::PathBuf) -> anyhow::Result<Self> {
        Ok(Self::run(std::fs::read_to_string(&path)?))
    }
    /// Runs `source` on its own thread (the console's Run).
    pub fn run(source: String) -> Self {
        let (tx, requests) = channel::<(Request, Sender<Reply>)>();
        let (done, finished) = channel();
        let (out, output) = channel::<String>();
        let stop = Arc::new(AtomicBool::new(false));
        let stopping = stop.clone();
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
            engine.on_print(move |s| {
                println!("{s}");
                let _ = out.send(s.to_string());
            });
            engine.on_progress(move |_| stopping.load(Ordering::Relaxed).then(|| "stopped".into()));
            let c = call.clone();
            engine.register_fn("add", move |json: &str| {
                done_or(c(Request::Add(json.into()))?)
            });
            let c = call.clone();
            engine.register_fn("clear", move || done_or(c(Request::Clear)?));
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
            let c = call.clone();
            engine.register_fn("get", move |k: &str| -> Res<String> {
                match c(Request::Get(k.into()))? {
                    Reply::Text(t) => Ok(t),
                    Reply::Done(Err(e)) => Err(e.into()),
                    _ => Ok(String::new()),
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
        Self {
            requests,
            finished,
            waiting: None,
            stale: false,
            output,
            stop,
        }
    }
    /// Ends the script: it stops at its next operation, or with an error from a call it is
    /// waiting on (the pending reply is dropped).
    pub fn stop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        self.waiting = None;
    }
    pub fn stopped(&self) -> bool {
        self.stop.load(Ordering::Relaxed)
    }
    /// The lines printed since the last call.
    pub fn output(&self) -> Vec<String> {
        self.output.try_iter().collect()
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

/// The last per-frame script error (None after a clean run), for the console.
pub static FRAME_ERROR: Mutex<Option<String>> = Mutex::new(None);

type Shared = Arc<Mutex<serde_json::Value>>;
thread_local! {
    /// The per-frame engine (its functions edit the shared settings JSON) and the last compiled
    /// script with its source.
    static FRAME: std::cell::RefCell<Option<(rhai::Engine, Shared, String, rhai::AST)>> =
        const { std::cell::RefCell::new(None) };
}
fn frame_engine(state: &Shared) -> rhai::Engine {
    let mut engine = rhai::Engine::new();
    // A runaway loop must not hang the app: about a few ms of work at most.
    engine.set_max_operations(2_000_000);
    engine.on_print(|s| println!("{s}"));
    fn put(state: &Shared, key: &str, value: serde_json::Value) -> Res<()> {
        let mut j = state.lock().unwrap_or_else(|e| e.into_inner());
        let slot = j
            .pointer_mut(&format!("/{}", key.trim_start_matches('/')))
            .ok_or_else(|| format!("unknown setting {key}"))?;
        *slot = value;
        Ok(())
    }
    let s = state.clone();
    engine.register_fn("set", move |k: &str, v: f64| {
        put(
            &s,
            k,
            serde_json::Number::from_f64(v).map_or(serde_json::Value::Null, Into::into),
        )
    });
    let s = state.clone();
    engine.register_fn("set", move |k: &str, v: i64| put(&s, k, v.into()));
    let s = state.clone();
    engine.register_fn("set", move |k: &str, v: bool| put(&s, k, v.into()));
    let s = state.clone();
    engine.register_fn("set", move |k: &str, v: &str| {
        put(&s, k, serde_json::from_str(v).unwrap_or_else(|_| v.into()))
    });
    let s = state.clone();
    engine.register_fn("get", move |k: &str| -> Res<rhai::Dynamic> {
        let j = s.lock().unwrap_or_else(|e| e.into_inner());
        let v = j
            .pointer(&format!("/{}", k.trim_start_matches('/')))
            .ok_or_else(|| format!("unknown setting {k}"))?;
        Ok(match v {
            serde_json::Value::Number(n) if n.is_f64() => n.as_f64().unwrap_or(0.).into(),
            serde_json::Value::Number(n) => n.as_i64().unwrap_or(0).into(),
            serde_json::Value::Bool(b) => (*b).into(),
            serde_json::Value::String(t) => t.clone().into(),
            other => other.to_string().into(),
        })
    });
    engine
}
/// Runs an animation's per-frame script (`Animation::script`) on the settings as JSON, at time
/// `t` (seconds) and `frame`: `set(key, value)` and `get(key)` reach any setting by its `/`
/// path, as tracks do; `t`, `frame` and `fps` are variables. It runs after the tracks, so it
/// can override or add to them; the result still goes through `validate()`.
pub fn per_frame(
    source: &str,
    v: &mut serde_json::Value,
    t: f32,
    frame: u32,
    fps: u32,
) -> Result<(), String> {
    FRAME.with(|cell| {
        let mut cell = cell.borrow_mut();
        if cell.as_ref().is_none_or(|c| c.2 != source) {
            let state: Shared = Arc::new(Mutex::new(serde_json::Value::Null));
            let engine = frame_engine(&state);
            let ast = engine.compile(source).map_err(|e| e.to_string())?;
            *cell = Some((engine, state, source.to_string(), ast));
        }
        let (engine, state, _, ast) = cell.as_ref().unwrap();
        *state.lock().unwrap_or_else(|e| e.into_inner()) = std::mem::take(v);
        let mut scope = rhai::Scope::new();
        scope.push("t", t as f64);
        scope.push("frame", frame as i64);
        scope.push("fps", fps as i64);
        let result = engine
            .run_ast_with_scope(&mut scope, ast)
            .map_err(|e| e.to_string());
        *v = std::mem::take(&mut *state.lock().unwrap_or_else(|e| e.into_inner()));
        result
    })
}

/// A top-level setting or a `/` path inside the settings, as JSON text.
pub fn get(s: &crate::scene::Settings, key: &str) -> anyhow::Result<String> {
    let j = serde_json::to_value(s)?;
    let pointer = format!("/{}", key.trim_start_matches('/'));
    j.pointer(&pointer)
        .map(|v| v.to_string())
        .ok_or_else(|| anyhow::anyhow!("unknown setting {key}"))
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
        assert_eq!(get(&s, "objects/0/rotation").unwrap(), "33.0");
        assert!(get(&s, "no_such_key").is_err());
    }

    #[test]
    fn a_per_frame_script_drives_settings() {
        let mut s = crate::scene::Settings::default();
        s.animation.script =
            "set(\"objects/0/rotation\", t * 90.0); if frame > 10 { set(\"spp\", 2); }".into();
        let a = s.animation.clone();
        let at = a.apply(&s, 0.2);
        assert_eq!(at.objects[0].rotation, 18.);
        assert_eq!(at.spp, s.spp, "frame 6 at 30 fps");
        let later = a.apply(&s, 1.);
        assert_eq!(later.spp, 2, "frame 30");
        // A bad script leaves the settings as the tracks made them and reports the error.
        s.animation.script = "set(\"no_such\", 1);".into();
        let a = s.animation.clone();
        assert_eq!(a.apply(&s, 0.5).objects[0].rotation, s.objects[0].rotation);
        assert!(FRAME_ERROR.lock().unwrap().is_some());
        // A value of the wrong type keeps the keyframes' result and reports why.
        s.animation.key("objects/0/rotation", 0., 10.);
        s.animation.script = "set(\"spp\", 2.5);".into();
        let a = s.animation.clone();
        assert_eq!(
            a.apply(&s, 0.5).objects[0].rotation,
            10.,
            "the keys still apply"
        );
        assert!(
            FRAME_ERROR
                .lock()
                .unwrap()
                .as_ref()
                .is_some_and(|e| e.contains("invalid"))
        );
        s.animation.tracks.clear();
        s.animation.script = "loop {}".into();
        let a = s.animation.clone();
        a.apply(&s, 0.5);
        assert!(FRAME_ERROR.lock().unwrap().is_some(), "a runaway loop ends");
    }
}
