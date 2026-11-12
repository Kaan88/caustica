//! Keyframe animation (roadmap track 8). A track drives one numeric field of `Settings`, named
//! by its JSON path ("yaw", "objects/2/position/0", "lights/0/color/1"), so anything stored as a
//! float can be animated without per-field code: camera, object transforms, lights, materials,
//! exposure. Booleans, integers and choices (enum names) are keyed too and hold each key's value
//! until the next (light on / off, a shape, the tone map); free text such as names and file
//! paths is not. The animation lives in the scene file (`Settings::animation`); the playhead
//! lives in the app (`Timeline`).
use crate::scene::Settings;
use glam::Vec3;
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// How a segment runs from its key to the next one.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum Interp {
    /// Monotone cubic through the neighbouring keys (Fritsch–Carlson): smooth, but never
    /// overshoots the keyed values, so they stay inside `validate()`'s ranges.
    #[default]
    Smooth,
    Linear,
    /// Starts and stops at rest (smoothstep).
    Ease,
    /// Keeps the key's value until the next key.
    Hold,
}
impl Interp {
    pub const ALL: [Interp; 4] = [Interp::Smooth, Interp::Linear, Interp::Ease, Interp::Hold];
    pub fn label(self) -> &'static str {
        match self {
            Interp::Smooth => "Smooth",
            Interp::Linear => "Linear",
            Interp::Ease => "Ease in and out",
            Interp::Hold => "Hold",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Key {
    pub time: f32,
    pub value: f64,
    #[serde(default)]
    pub interp: Interp,
    /// A choice's name (an enum field's JSON string); `value` is unused then.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Track {
    pub path: String,
    /// Sorted by time, at least one.
    pub keys: Vec<Key>,
}
impl Track {
    /// The key in force at `t` for fields that don't interpolate: the last at or before `t`,
    /// else the first.
    pub fn held(&self, t: f32) -> Option<&Key> {
        let i = self.keys.partition_point(|k| k.time <= t + 1e-6);
        self.keys.get(i.saturating_sub(1))
    }
    pub fn value(&self, t: f32) -> f64 {
        let k = &self.keys;
        let Some(first) = k.first() else { return 0. };
        if t <= first.time || k.len() == 1 {
            return first.value;
        }
        let last = k.len() - 1;
        if t >= k[last].time {
            return k[last].value;
        }
        let i = k.partition_point(|key| key.time <= t) - 1;
        let (a, b) = (&k[i], &k[i + 1]);
        let h = (b.time - a.time).max(1e-6) as f64;
        let u = ((t - a.time) as f64 / h).clamp(0., 1.);
        match a.interp {
            Interp::Hold => a.value,
            Interp::Linear => a.value + (b.value - a.value) * u,
            Interp::Ease => a.value + (b.value - a.value) * u * u * (3. - 2. * u),
            Interp::Smooth => {
                let (m0, m1) = (self.slope(i), self.slope(i + 1));
                let (u2, u3) = (u * u, u * u * u);
                (2. * u3 - 3. * u2 + 1.) * a.value
                    + (u3 - 2. * u2 + u) * h * m0
                    + (-2. * u3 + 3. * u2) * b.value
                    + (u3 - u2) * h * m1
            }
        }
    }
    /// Fritsch–Carlson tangent at key i: zero at extrema and at the ends' flat sides, limited so
    /// each segment stays monotone.
    fn slope(&self, i: usize) -> f64 {
        let k = &self.keys;
        let secant = |j: usize| {
            (k[j + 1].value - k[j].value) / ((k[j + 1].time - k[j].time).max(1e-6) as f64)
        };
        let n = k.len();
        if n < 2 {
            return 0.;
        }
        let (left, right) = (
            (i > 0).then(|| secant(i - 1)),
            (i + 1 < n).then(|| secant(i)),
        );
        match (left, right) {
            (Some(l), Some(r)) => {
                if l * r <= 0. {
                    0.
                } else {
                    // The harmonic mean keeps both neighbouring segments monotone.
                    2. * l * r / (l + r)
                }
            }
            // The ends start and stop at rest, as most camera moves want.
            _ => 0.,
        }
    }
    fn insert_text(&mut self, time: f32, value: f64, text: Option<String>) {
        match self.keys.iter_mut().find(|k| (k.time - time).abs() < 1e-4) {
            Some(k) => {
                k.value = value;
                k.text = text;
            }
            None => {
                let at = self.keys.partition_point(|k| k.time < time);
                // A new key takes the interpolation of the segment it splits.
                let interp = at
                    .checked_sub(1)
                    .map_or(Interp::default(), |i| self.keys[i].interp);
                self.keys.insert(
                    at,
                    Key {
                        time,
                        value,
                        interp,
                        text,
                    },
                );
            }
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Animation {
    /// Seconds; playback loops over it, and timeline exports render all of it.
    pub length: f32,
    pub fps: u32,
    /// Motion blur in timeline exports: the shutter's open time as a share of a frame
    /// (0 off, 0.5 = a 180° shutter).
    pub shutter: f32,
    pub tracks: Vec<Track>,
    /// Camera path: between the camera's keys the eye and its look-at target move along
    /// centripetal Catmull-Rom splines through their keyed positions (`camera_on_path`),
    /// instead of yaw, pitch, distance and target each interpolating alone (which swings the
    /// eye around the target).
    pub camera_path: bool,
    /// A Rhai script run on every frame after the tracks (`script::per_frame`): `set(key,
    /// value)` and `get(key)` on any setting by its `/` path, with `t`, `frame` and `fps`.
    pub script: String,
}
impl Default for Animation {
    fn default() -> Self {
        Self {
            length: 5.,
            fps: 30,
            shutter: 0.,
            tracks: vec![],
            camera_path: false,
            script: String::new(),
        }
    }
}
/// The camera's orbit fields, which a camera path drives together.
const CAMERA: [&str; 6] = [
    "yaw", "pitch", "distance", "target/0", "target/1", "target/2",
];
/// Centripetal Catmull-Rom between p[1] and p[2] at u ∈ [0, 1] (Barry–Goldman pyramid).
fn catmull_rom(p: [Vec3; 4], u: f32) -> Vec3 {
    let knot = |a: Vec3, b: Vec3| a.distance(b).sqrt().max(1e-4);
    let t1 = knot(p[0], p[1]);
    let t2 = t1 + knot(p[1], p[2]);
    let t3 = t2 + knot(p[2], p[3]);
    let t = t1 + (t2 - t1) * u;
    let mix = |a: Vec3, b: Vec3, ta: f32, tb: f32| {
        a * ((tb - t) / (tb - ta)) + b * ((t - ta) / (tb - ta))
    };
    let a1 = mix(p[0], p[1], 0., t1);
    let a2 = mix(p[1], p[2], t1, t2);
    let a3 = mix(p[2], p[3], t2, t3);
    let b1 = mix(a1, a2, 0., t2);
    let b2 = mix(a2, a3, t1, t3);
    mix(b1, b2, t1, t2)
}
/// Points the orbit camera from `eye` at `target`: yaw is kept within half a turn of the
/// per-field value (so a camera that has turned several times stays continuous), the distance
/// and pitch inside `validate()`'s ranges.
fn set_orbit(s: &mut Settings, eye: Vec3, target: Vec3) {
    let d = eye - target;
    let len = d.length().max(1e-6);
    let mut yaw = d.x.atan2(d.z).to_degrees();
    yaw += 360. * ((s.yaw - yaw) / 360.).round();
    s.yaw = yaw;
    s.pitch = (d.y / len)
        .clamp(-1., 1.)
        .asin()
        .to_degrees()
        .clamp(-89., 89.);
    s.distance = len.clamp(0.5, 200.);
    s.target = target.to_array();
}
/// Top-level fields that are floats but not part of the picture: render scale and HDR display
/// levels, the filter width and UI-only values.
const NOT_ANIMATED: [&str; 7] = [
    "scale",
    "hdr_peak",
    "hdr_white",
    "filter_width",
    "split",
    "rotate_speed",
    "animation",
];
/// Whether `path` names a field a track may drive (it must also be a float in the settings).
pub fn animatable(path: &str) -> bool {
    let top = path.split('/').next().unwrap_or("");
    !NOT_ANIMATED.contains(&top) && !path.is_empty()
}
impl Animation {
    pub fn is_empty(&self) -> bool {
        self.tracks.is_empty() && self.script.trim().is_empty()
    }
    /// Scene files leave out an untouched animation.
    pub fn is_default(&self) -> bool {
        *self == Self::default()
    }
    pub fn validate(&self) -> anyhow::Result<()> {
        anyhow::ensure!(
            (0.1..=3600.).contains(&self.length)
                && (1..=240).contains(&self.fps)
                && (0. ..=1.).contains(&self.shutter),
            "Invalid animation length, fps or shutter"
        );
        anyhow::ensure!(self.tracks.len() <= 100_000, "Too many animation tracks");
        anyhow::ensure!(self.script.len() <= 65536, "Per-frame script too long");
        for t in &self.tracks {
            anyhow::ensure!(
                !t.keys.is_empty()
                    && t.path.len() <= 256
                    && t.keys.iter().all(|k| k.time.is_finite()
                        && k.value.is_finite()
                        && k.text.as_ref().is_none_or(|s| s.len() <= 64))
                    && t.keys.windows(2).all(|w| w[0].time < w[1].time),
                "Invalid animation track {}",
                t.path
            );
        }
        Ok(())
    }
    /// Frames in a full timeline export.
    pub fn frames(&self) -> u32 {
        ((self.length * self.fps as f32).round() as u32).max(1)
    }
    pub fn track(&self, path: &str) -> Option<&Track> {
        self.tracks.iter().find(|t| t.path == path)
    }
    /// `s` with every track's value at time `t`. Tracks whose field no longer exists (or is
    /// no longer a float) are skipped; if the result fails `validate()`, `s` comes back as is.
    pub fn apply(&self, s: &Settings, t: f32) -> Settings {
        if self.is_empty() {
            return s.clone();
        }
        let Ok(mut v) = serde_json::to_value(s) else {
            return s.clone();
        };
        for track in self.tracks.iter().filter(|t| animatable(&t.path)) {
            let Some(slot) = v.pointer_mut(&format!("/{}", track.path)) else {
                continue;
            };
            let held = track.held(t);
            match slot {
                Value::Number(n) if n.is_f64() => {
                    if let Some(n) = serde_json::Number::from_f64(track.value(t)) {
                        *slot = Value::Number(n);
                    }
                }
                Value::Number(n) => {
                    if let Some(k) = held.filter(|k| k.text.is_none()) {
                        let r = k.value.round();
                        *slot = Value::Number(if n.is_u64() {
                            (r.max(0.) as u64).into()
                        } else {
                            (r as i64).into()
                        });
                    }
                }
                Value::Bool(_) => {
                    if let Some(k) = held.filter(|k| k.text.is_none()) {
                        *slot = Value::Bool(k.value >= 0.5);
                    }
                }
                Value::String(_) => {
                    if let Some(text) = held.and_then(|k| k.text.clone()) {
                        *slot = Value::String(text);
                    }
                }
                _ => {}
            }
        }
        let finish = |v: Value| -> anyhow::Result<Settings> {
            let mut out: Settings = serde_json::from_value(v)?;
            if let Some((eye, target)) = self.camera_on_path(s, t) {
                set_orbit(&mut out, eye, target);
            }
            out.validate()?;
            Ok(out)
        };
        if self.script.trim().is_empty() {
            return finish(v).unwrap_or_else(|_| s.clone());
        }
        // The per-frame script runs on the tracks' result; if it fails, or sets a value of the
        // wrong type or out of range, the frame keeps the tracks' result and the console shows
        // why (a failing script must not silently drop the keyframes too).
        let tracks_only = v.clone();
        let frame = (t * self.fps as f32).round().max(0.) as u32;
        let error = match crate::script::per_frame(&self.script, &mut v, t, frame, self.fps) {
            Err(e) => Some(e),
            Ok(()) => match finish(v) {
                Ok(out) => {
                    *crate::script::FRAME_ERROR
                        .lock()
                        .unwrap_or_else(|e| e.into_inner()) = None;
                    return out;
                }
                Err(e) => Some(format!("the script set an invalid value: {e:#}")),
            },
        };
        *crate::script::FRAME_ERROR
            .lock()
            .unwrap_or_else(|e| e.into_inner()) = error;
        finish(tracks_only).unwrap_or_else(|_| s.clone())
    }
    /// The camera's key times (any of its orbit fields), sorted and deduplicated.
    fn camera_times(&self) -> Vec<f32> {
        let mut t: Vec<f32> = self
            .tracks
            .iter()
            .filter(|tr| CAMERA.contains(&tr.path.as_str()))
            .flat_map(|tr| tr.keys.iter().map(|k| k.time))
            .collect();
        t.sort_by(f32::total_cmp);
        t.dedup_by(|a, b| (*a - *b).abs() < 1e-4);
        t
    }
    /// The eye and target the camera's tracks give at `t` (fields without a track from `s`).
    fn orbit_at(&self, s: &Settings, t: f32) -> (Vec3, Vec3) {
        let f = |path: &str, base: f32| self.track(path).map_or(base, |tr| tr.value(t) as f32);
        let target = Vec3::new(
            f("target/0", s.target[0]),
            f("target/1", s.target[1]),
            f("target/2", s.target[2]),
        );
        let (sy, cy) = f("yaw", s.yaw).to_radians().sin_cos();
        let (sp, cp) = f("pitch", s.pitch).to_radians().sin_cos();
        let eye = target + Vec3::new(sy * cp, sp, cy * cp) * f("distance", s.distance);
        (eye, target)
    }
    /// Camera path (`camera_path`): the eye and target at `t` on centripetal Catmull-Rom splines
    /// (Barry–Goldman, α = 0.5: no cusps or self-loops between close keys) through their
    /// positions at the camera's key times, the ends extended by reflection. A segment keyed
    /// Ease starts and stops at rest; Hold keeps the key. None outside the keys, at Hold, or
    /// with fewer than two camera keys (the per-field values are the same there).
    pub fn camera_on_path(&self, s: &Settings, t: f32) -> Option<(Vec3, Vec3)> {
        if !self.camera_path {
            return None;
        }
        self.on_path(s, t, &self.camera_times())
    }
    /// `camera_on_path` with the camera's key times given.
    fn on_path(&self, s: &Settings, t: f32, times: &[f32]) -> Option<(Vec3, Vec3)> {
        let n = times.len();
        if n < 2 || t <= times[0] || t >= times[n - 1] {
            return None;
        }
        let i = times.partition_point(|k| *k <= t) - 1;
        let interp = CAMERA
            .iter()
            .filter_map(|p| self.track(p))
            .find_map(|tr| tr.keys.iter().find(|k| (k.time - times[i]).abs() < 1e-4))
            .map_or(Interp::Smooth, |k| k.interp);
        let u = (t - times[i]) / (times[i + 1] - times[i]).max(1e-6);
        let u = match interp {
            Interp::Hold => return None,
            Interp::Ease => u * u * (3. - 2. * u),
            Interp::Smooth | Interp::Linear => u,
        };
        let at = |j: usize| self.orbit_at(s, times[j]);
        let (p1, p2) = (at(i), at(i + 1));
        let p0 = if i > 0 {
            at(i - 1)
        } else {
            (2. * p1.0 - p2.0, 2. * p1.1 - p2.1)
        };
        let p3 = if i + 2 < n {
            at(i + 2)
        } else {
            (2. * p2.0 - p1.0, 2. * p2.1 - p1.1)
        };
        Some((
            catmull_rom([p0.0, p1.0, p2.0, p3.0], u),
            catmull_rom([p0.1, p1.1, p2.1, p3.1], u),
        ))
    }
    /// The eye's path over the camera's keys, `samples` points per segment (for the viewport).
    pub fn camera_path_points(&self, s: &Settings, samples: usize) -> Vec<Vec3> {
        let times = self.camera_times();
        if times.len() < 2 {
            return vec![];
        }
        let mut out = vec![];
        for w in times.windows(2) {
            for k in 0..samples {
                let t = w[0] + (w[1] - w[0]) * k as f32 / samples as f32;
                let on = self.camera_path.then(|| self.on_path(s, t, &times));
                out.push(on.flatten().map_or_else(|| self.orbit_at(s, t).0, |p| p.0));
            }
        }
        out.push(self.orbit_at(s, times[times.len() - 1]).0);
        out
    }
    /// Sets a key on `path` at `t` (replacing one at the same time).
    pub fn key(&mut self, path: &str, t: f32, value: f64) {
        self.key_leaf(path, t, &Leaf::Number(value));
    }
    /// Sets a key of any keyable kind; discrete ones start on Hold.
    pub fn key_leaf(&mut self, path: &str, t: f32, leaf: &Leaf) {
        let (value, text) = match leaf {
            Leaf::Number(v) | Leaf::Discrete(v) => (*v, None),
            Leaf::Text(s) => (0., Some(s.clone())),
        };
        match self.tracks.iter_mut().find(|tr| tr.path == path) {
            Some(tr) => tr.insert_text(t, value, text),
            None => self.tracks.push(Track {
                path: path.into(),
                keys: vec![Key {
                    time: t,
                    value,
                    interp: if matches!(leaf, Leaf::Number(_)) {
                        Interp::default()
                    } else {
                        Interp::Hold
                    },
                    text,
                }],
            }),
        }
    }
    /// Removes the keys at `t` from the tracks under `prefixes` (empty = all); empty tracks go.
    /// Returns how many keys were removed.
    pub fn remove_keys(&mut self, prefixes: &[String], t: f32) -> usize {
        let mut removed = 0;
        for tr in self
            .tracks
            .iter_mut()
            .filter(|tr| under_any(&tr.path, prefixes))
        {
            let before = tr.keys.len();
            tr.keys.retain(|k| (k.time - t).abs() >= 1e-4);
            removed += before - tr.keys.len();
        }
        self.tracks.retain(|tr| !tr.keys.is_empty());
        removed
    }
    /// Sets the interpolation of the keys at `t` under `prefixes`.
    pub fn set_interp(&mut self, prefixes: &[String], t: f32, interp: Interp) -> usize {
        let mut n = 0;
        for tr in self
            .tracks
            .iter_mut()
            .filter(|tr| under_any(&tr.path, prefixes))
        {
            for k in tr.keys.iter_mut().filter(|k| (k.time - t).abs() < 1e-4) {
                k.interp = interp;
                n += 1;
            }
        }
        n
    }
    /// Retimes the keys under `prefixes` about `pivot`: t → pivot + (t − pivot) · scale +
    /// offset, clamped to 0 and the length (which grows to fit). Keys that land on the same
    /// time merge (the later one wins). Returns how many keys moved.
    pub fn retime(&mut self, prefixes: &[String], pivot: f32, scale: f32, offset: f32) -> usize {
        let mut n = 0;
        let mut end: f32 = self.length;
        for tr in self
            .tracks
            .iter_mut()
            .filter(|tr| under_any(&tr.path, prefixes))
        {
            let keys = std::mem::take(&mut tr.keys);
            for mut k in keys {
                let t = (pivot + (k.time - pivot) * scale + offset).clamp(0., 3600.);
                n += usize::from((t - k.time).abs() > 1e-6);
                end = end.max(t);
                k.time = t;
                match tr.keys.iter_mut().find(|x| (x.time - t).abs() < 1e-4) {
                    Some(x) => *x = k,
                    None => tr.keys.push(k),
                }
            }
            tr.keys.sort_by(|a, b| a.time.total_cmp(&b.time));
        }
        self.length = end;
        n
    }
    /// Copies the tracks under `from` (an object's or light's prefix) to `to`, replacing
    /// `to`'s own. Returns the number of tracks copied.
    pub fn copy_keys(&mut self, from: &str, to: &str) -> usize {
        if from == to || from.is_empty() || to.is_empty() {
            return 0;
        }
        self.tracks.retain(|tr| !under(&tr.path, to));
        let copies: Vec<Track> = self
            .tracks
            .iter()
            .filter(|tr| under(&tr.path, from))
            .map(|tr| Track {
                path: format!("{to}{}", &tr.path[from.len()..]),
                keys: tr.keys.clone(),
            })
            .collect();
        let n = copies.len();
        self.tracks.extend(copies);
        n
    }
    /// Key times under `prefixes` (empty = all), sorted and deduplicated.
    pub fn times(&self, prefixes: &[String]) -> Vec<f32> {
        let mut t: Vec<f32> = self
            .tracks
            .iter()
            .filter(|tr| under_any(&tr.path, prefixes))
            .flat_map(|tr| tr.keys.iter().map(|k| k.time))
            .collect();
        t.sort_by(f32::total_cmp);
        t.dedup_by(|a, b| (*a - *b).abs() < 1e-4);
        t
    }
    /// The interpolation shared by the keys at `t` under `prefixes`, if they agree.
    pub fn interp_at(&self, prefixes: &[String], t: f32) -> Option<Interp> {
        let mut found = None;
        for tr in self
            .tracks
            .iter()
            .filter(|tr| under_any(&tr.path, prefixes))
        {
            for k in tr.keys.iter().filter(|k| (k.time - t).abs() < 1e-4) {
                match found {
                    None => found = Some(k.interp),
                    Some(i) if i != k.interp => return None,
                    _ => {}
                }
            }
        }
        found
    }
    /// After `objects[index]` (or `lights[index]`) was removed: its tracks go, later indices
    /// shift down.
    pub fn removed(&mut self, list: &str, index: usize) {
        let prefix = format!("{list}/");
        self.tracks.retain_mut(|tr| {
            let Some(rest) = tr.path.strip_prefix(&prefix) else {
                return true;
            };
            let (i, tail) = rest.split_once('/').unwrap_or((rest, ""));
            match i.parse::<usize>() {
                Ok(i) if i == index => false,
                Ok(i) if i > index => {
                    tr.path = format!("{prefix}{}/{tail}", i - 1);
                    true
                }
                _ => true,
            }
        });
    }
}
/// `path` is `prefix` or lies under it ("objects/1" covers "objects/1/position/0").
pub fn under(path: &str, prefix: &str) -> bool {
    prefix.is_empty()
        || path == prefix
        || (path.starts_with(prefix) && path.as_bytes().get(prefix.len()) == Some(&b'/'))
}
/// `path` lies under one of `prefixes`, or `prefixes` is empty (everything).
pub fn under_any(path: &str, prefixes: &[String]) -> bool {
    prefixes.is_empty() || prefixes.iter().any(|p| under(path, p))
}
/// A keyable value: a float (interpolated), a boolean or integer (held), or a choice's name.
#[derive(Clone, Debug, PartialEq)]
pub enum Leaf {
    Number(f64),
    Discrete(f64),
    Text(String),
}
/// Fields under these names hold free text or references, not choices to key.
const NOT_KEYED: [&str; 12] = [
    "name",
    "mesh",
    "texture",
    "maps_from",
    "scene_name",
    "hdri",
    "lpe",
    "title",
    "portal",
    "target",
    "targets",
    "object",
];
/// A string that looks like an enum's JSON name (snake case).
fn choice(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 32
        && s.bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_')
}
/// Every keyable field under `prefix`: floats, booleans, integers and choices.
pub fn keyable(v: &Value, prefix: &str) -> Vec<(String, Leaf)> {
    fn walk(v: &Value, path: String, out: &mut Vec<(String, Leaf)>) {
        // Any segment: `puzzle/targets/0` is a reference too.
        let free = path.split('/').any(|seg| NOT_KEYED.contains(&seg)) || !animatable(&path);
        match v {
            Value::Number(n) if n.is_f64() => {
                if animatable(&path) {
                    out.push((path, Leaf::Number(n.as_f64().unwrap_or(0.))));
                }
            }
            Value::Number(n) if !free => {
                out.push((path, Leaf::Discrete(n.as_f64().unwrap_or(0.))));
            }
            Value::Bool(b) if !free => out.push((path, Leaf::Discrete(*b as u8 as f64))),
            Value::String(s) if !free && choice(s) => out.push((path, Leaf::Text(s.clone()))),
            Value::Array(a) => {
                for (i, x) in a.iter().enumerate() {
                    walk(x, join(&path, &i.to_string()), out);
                }
            }
            Value::Object(o) => {
                for (k, x) in o {
                    walk(x, join(&path, k), out);
                }
            }
            _ => {}
        }
    }
    fn join(a: &str, b: &str) -> String {
        if a.is_empty() {
            b.into()
        } else {
            format!("{a}/{b}")
        }
    }
    let mut out: Vec<(String, Leaf)> = vec![];
    let start = if prefix.is_empty() {
        Some(v)
    } else {
        v.pointer(&format!("/{prefix}"))
    };
    if let Some(start) = start {
        walk(start, prefix.into(), &mut out);
    }
    out
}

/// The playhead and the state of a key edit (in the app, not the scene).
pub struct Timeline {
    pub time: f32,
    pub playing: bool,
    pub looping: bool,
    /// The settings as the animation last left them: edits since then are what I and Shift+I
    /// key. None while the scene has no keys (then nothing counts as edited).
    pub rest: Option<Settings>,
    /// The time, tracks, camera-path switch and per-frame script `rest` was evaluated at; the
    /// animation is applied again when any of them changes (not for length, fps or shutter,
    /// which move no key).
    pub applied: Option<(f32, Vec<Track>, bool, String)>,
    /// Auto-key: every finished edit at a still playhead is keyed (Shift+I by itself).
    pub auto_key: bool,
    /// The curve editor is open (below the strip).
    pub curves: bool,
    /// The key being dragged in the curve editor: its track, its index, and the value range
    /// the editor's height stood for when the drag began.
    pub curve_drag: Option<(String, usize, f64, f64)>,
}
impl Default for Timeline {
    fn default() -> Self {
        Self {
            time: 0.,
            playing: false,
            looping: true,
            rest: None,
            applied: None,
            auto_key: false,
            curves: false,
            curve_drag: None,
        }
    }
}
impl Timeline {
    /// Moves the playhead by `dt` while playing; at the end it loops or stops.
    pub fn advance(&mut self, a: &Animation, dt: f32) {
        if !self.playing {
            return;
        }
        self.time += dt;
        if self.time > a.length {
            if self.looping {
                self.time = self.time.rem_euclid(a.length.max(1e-3));
            } else {
                self.time = a.length;
                self.playing = false;
            }
        }
    }
    /// The keys differ from the ones last applied or settled (or nothing was applied yet).
    pub fn keys_changed(&self, s: &Settings) -> bool {
        self.applied.as_ref().is_none_or(|(_, a, path, script)| {
            *a != s.animation.tracks
                || *path != s.animation.camera_path
                || *script != s.animation.script
        })
    }
    /// The playhead left the time last applied or settled.
    pub fn playhead_moved(&self) -> bool {
        self.applied.as_ref().is_some_and(|(t, ..)| *t != self.time)
    }
    /// Applies the animation when the playhead or the keys changed since the last time; edits
    /// at a still playhead stay until it moves (as in Blender), so they can be keyed.
    pub fn update(&mut self, s: &mut Settings) -> bool {
        // Without keys nothing is overridden, so nothing counts as an edit to key.
        if s.animation.is_empty() {
            self.rest = None;
        }
        if !self.keys_changed(s) && !self.playhead_moved() {
            // Objects or lights added or removed since: their indices no longer match `rest`.
            if let Some(r) = &self.rest
                && (r.objects.len() != s.objects.len() || r.lights.len() != s.lights.len())
            {
                self.rest = Some(s.clone());
            }
            return false;
        }
        if !s.animation.is_empty() {
            *s = s.animation.apply(s, self.time);
            self.rest = Some(s.clone());
        }
        self.applied = Some((
            self.time,
            s.animation.tracks.clone(),
            s.animation.camera_path,
            s.animation.script.clone(),
        ));
        true
    }
    /// Takes `s` as the animation's current result without applying it (after keying, undo and
    /// redo, whose values must stay as they are).
    pub fn settle(&mut self, s: &Settings) {
        self.rest = (!s.animation.is_empty()).then(|| s.clone());
        self.applied = Some((
            self.time,
            s.animation.tracks.clone(),
            s.animation.camera_path,
            s.animation.script.clone(),
        ));
    }
    /// Auto-key: keys what changed since the last applied or keyed state, once no edit is in
    /// progress. Before the scene's first key, the state when auto-key was turned on (or the
    /// last frame's) is the reference. Returns the number of keys set.
    pub fn auto_key(&mut self, s: &mut Settings, busy: bool, last: &mut Option<Settings>) -> usize {
        if !self.auto_key || self.playing {
            *last = None;
            return 0;
        }
        if busy {
            return 0;
        }
        if self.rest.is_none() {
            self.rest = last.take();
        }
        let n = match &self.rest {
            Some(r) if r != s => key_changed(s, self),
            _ => 0,
        };
        if n > 0 {
            self.settle(s);
        }
        if last.as_ref() != Some(s) {
            *last = Some(s.clone());
        }
        n
    }
    /// Auto-key: takes `s` as the reference without keying (after the app's own changes:
    /// physics, walking, the remote).
    pub fn absorb(&mut self, s: &Settings, last: &mut Option<Settings>) {
        if !self.auto_key || self.playing {
            return;
        }
        if self.rest.as_ref().is_some_and(|r| r != s) {
            self.settle(s);
        }
        if last.as_ref() != Some(s) {
            *last = Some(s.clone());
        }
    }
    /// Snaps a time to the animation's frame grid.
    pub fn snap(a: &Animation, t: f32) -> f32 {
        let fps = a.fps.max(1) as f32;
        ((t * fps).round() / fps).clamp(0., a.length)
    }
}

/// The paths a selection owns: its subtree for an object or light, its fields for the camera
/// (also with nothing selected) and the main light.
pub fn prefixes(selection: crate::ui::Selection) -> Vec<String> {
    let (prefix, fields) = entity(selection);
    if prefix.is_empty() {
        fields.iter().map(|f| f.to_string()).collect()
    } else {
        vec![prefix]
    }
}
/// The JSON prefix of a selection's own fields, and the fields an entity's first key covers.
pub fn entity(selection: crate::ui::Selection) -> (String, &'static [&'static str]) {
    use crate::ui::Selection;
    const CAMERA: &[&str] = &[
        "yaw",
        "pitch",
        "distance",
        "target",
        "fov",
        "focus_distance",
    ];
    const OBJECT: &[&str] = &[
        "position",
        "rotation",
        "tilt",
        "size",
        "stretch",
        "scale_xz",
        "elevation",
    ];
    const LIGHT: &[&str] = &["position", "rotation", "size", "intensity"];
    const MAIN: &[&str] = &[
        "light_position",
        "light_size",
        "light_aspect",
        "light_intensity",
    ];
    match selection {
        Selection::Object(i) => (format!("objects/{i}"), OBJECT),
        Selection::Light(i) => (format!("lights/{i}"), LIGHT),
        Selection::MainLight => (String::new(), MAIN),
        // It has no keys; without this it would fall through to the camera's.
        Selection::Flashlight => (String::new(), &[]),
        _ => (String::new(), CAMERA),
    }
}
/// Keys the selection at the playhead: the fields it already animates, those edited since the
/// animation last applied, and, on its first key, its transform (or the camera). Returns the
/// number of keys set.
pub fn key_selection(s: &mut Settings, tl: &Timeline, selection: crate::ui::Selection) -> usize {
    let (prefix, first) = entity(selection);
    let v = serde_json::to_value(&*s).unwrap_or(Value::Null);
    let rest = match &tl.rest {
        Some(r) => serde_json::to_value(r).unwrap_or(Value::Null),
        None => v.clone(),
    };
    // Top-level groups (camera, main light) list their fields; objects and lights cover all.
    let candidates: Vec<(String, Leaf)> = if prefix.is_empty() {
        first.iter().flat_map(|f| keyable(&v, f)).collect()
    } else {
        keyable(&v, &prefix)
    };
    let fresh = !candidates
        .iter()
        .any(|(p, _)| s.animation.track(p).is_some());
    let first_key = |p: &str| {
        first.iter().any(|f| {
            under(
                p,
                &if prefix.is_empty() {
                    f.to_string()
                } else {
                    format!("{prefix}/{f}")
                },
            )
        })
    };
    let mut n = 0;
    for (path, leaf) in candidates {
        let changed = rest
            .pointer(&format!("/{path}"))
            .is_some_and(|r| Some(r) != v.pointer(&format!("/{path}")));
        let floats_first = fresh && first_key(&path) && matches!(leaf, Leaf::Number(_));
        if s.animation.track(&path).is_some() || changed || floats_first {
            s.animation.key_leaf(&path, tl.time, &leaf);
            n += 1;
        }
    }
    n
}
/// Keys the value at `path` at `t` as it stands (an array's path keys its components): a
/// property row's key mark. Returns the number of keys set: none for a value that can't be
/// keyed, or that the scene's JSON leaves out at its default.
pub fn key_path(s: &mut Settings, t: f32, path: &str) -> usize {
    let v = serde_json::to_value(&*s).unwrap_or(Value::Null);
    let leaves = keyable(&v, path);
    for (p, leaf) in &leaves {
        s.animation.key_leaf(p, t, leaf);
    }
    leaves.len()
}
/// Keys every float edited since the animation last applied, anywhere in the scene.
pub fn key_changed(s: &mut Settings, tl: &Timeline) -> usize {
    let v = serde_json::to_value(&*s).unwrap_or(Value::Null);
    let rest = match &tl.rest {
        Some(r) => serde_json::to_value(r).unwrap_or(Value::Null),
        None => v.clone(),
    };
    let mut n = 0;
    for (path, leaf) in keyable(&v, "") {
        let changed = rest
            .pointer(&format!("/{path}"))
            .is_some_and(|r| Some(r) != v.pointer(&format!("/{path}")));
        if changed {
            s.animation.key_leaf(&path, tl.time, &leaf);
            n += 1;
        }
    }
    n
}

#[cfg(test)]
mod tests {
    use super::*;
    fn track(keys: &[(f32, f64, Interp)]) -> Track {
        Track {
            path: "yaw".into(),
            keys: keys
                .iter()
                .map(|&(time, value, interp)| Key {
                    time,
                    value,
                    interp,
                    text: None,
                })
                .collect(),
        }
    }
    #[test]
    fn a_camera_path_dollies_straight() {
        // Eye from (-2, 0, 2) to (2, 0, 2) looking at the origin: yaw −45° → 45°, distance √8.
        let mut s = Settings::default();
        s.animation.length = 2.;
        let d = 8f64.sqrt();
        for (t, yaw) in [(0., -45.), (2., 45.)] {
            s.animation.key("yaw", t, yaw);
            s.animation.key("pitch", t, 0.);
            s.animation.key("distance", t, d);
            for c in 0..3 {
                s.animation.key(&format!("target/{c}"), t, 0.);
            }
        }
        let eye = |s: &Settings| {
            let (sy, cy) = s.yaw.to_radians().sin_cos();
            let (sp, cp) = s.pitch.to_radians().sin_cos();
            Vec3::from(s.target) + Vec3::new(sy * cp, sp, cy * cp) * s.distance
        };
        // Per field, the eye swings around the target.
        let arc = eye(&s.animation.apply(&s, 1.));
        assert!((arc.z - d as f32).abs() < 1e-3, "{arc}");
        s.animation.camera_path = true;
        for t in [0.5, 1., 1.5] {
            let e = eye(&s.animation.apply(&s, t));
            assert!(
                (e.z - 2.).abs() < 1e-3 && e.y.abs() < 1e-3,
                "off the line at {t}: {e}"
            );
        }
        let mid = eye(&s.animation.apply(&s, 1.));
        assert!(mid.x.abs() < 1e-3, "{mid}");
        // The keys themselves are unchanged.
        let start = eye(&s.animation.apply(&s, 0.));
        assert!(start.distance(Vec3::new(-2., 0., 2.)) < 1e-3, "{start}");
        assert_eq!(s.animation.camera_path_points(&s, 8).len(), 9);
    }
    #[test]
    fn interpolations_hit_their_keys_and_smooth_never_overshoots() {
        use Interp::*;
        let t = track(&[
            (0., 0., Linear),
            (1., 10., Smooth),
            (2., 10.5, Hold),
            (3., 0., Ease),
        ]);
        assert_eq!(t.value(-1.), 0.);
        assert_eq!(t.value(0.5), 5.);
        assert_eq!(t.value(1.), 10.);
        assert_eq!(t.value(2.5), 10.5);
        assert_eq!(t.value(9.), 0.);
        // Smooth from 10 to 10.5 next to a peak: monotone, inside the keys.
        for i in 0..=100 {
            let v = t.value(1. + i as f32 / 100.);
            assert!((10. ..=10.5).contains(&v), "{v}");
        }
        let e = track(&[(0., 0., Ease), (1., 1., Ease)]);
        assert!((e.value(0.5) - 0.5).abs() < 1e-9 && e.value(0.1) < 0.1);
    }
    #[test]
    fn apply_drives_floats_and_holds_discrete_fields() {
        let mut s = Settings::default();
        s.animation.key("objects/0/position/0", 0., -0.5);
        s.animation.key("objects/0/position/0", 2., 0.5);
        s.animation.tracks[0].keys[0].interp = Interp::Linear;
        s.animation.key("bounces", 0., 2.);
        s.animation.key("objects/9/size", 0., 2.);
        let out = s.animation.apply(&s, 1.);
        assert!(out.objects[0].position[0].abs() < 1e-6);
        assert_eq!(out.bounces, 2, "integers hold their key");
        // Out-of-range values leave the settings alone.
        s.animation.key("exposure", 0., -1.);
        assert_eq!(
            s.animation.apply(&s, 1.).objects[0].position[0],
            s.objects[0].position[0]
        );
    }
    #[test]
    fn booleans_and_choices_hold_until_the_next_key() {
        let mut s = Settings::default();
        let mut tl = Timeline {
            rest: Some(s.clone()),
            ..Timeline::default()
        };
        s.light_on = false;
        s.objects[0].shape = crate::scene::Shape::Sphere;
        tl.time = 1.;
        assert!(key_changed(&mut s, &tl) >= 2);
        tl.rest = Some(s.clone());
        s.light_on = true;
        s.objects[0].shape = crate::scene::Shape::Box;
        tl.time = 2.;
        key_changed(&mut s, &tl);
        let a = &s.animation;
        let at = |t: f32| a.apply(&s, t);
        assert!(!at(1.5).light_on && at(2.).light_on && !at(0.).light_on);
        assert!(at(1.9).objects[0].shape == crate::scene::Shape::Sphere);
        assert!(at(2.5).objects[0].shape == crate::scene::Shape::Box);
        // Names are never keyed.
        assert!(a.tracks.iter().all(|t| !t.path.ends_with("name")));
    }
    #[test]
    fn auto_key_keys_finished_edits() {
        let mut s = Settings::default();
        let mut tl = Timeline {
            auto_key: true,
            ..Timeline::default()
        };
        let mut last = None;
        assert_eq!(tl.auto_key(&mut s, false, &mut last), 0);
        s.exposure = 1.5;
        assert_eq!(tl.auto_key(&mut s, true, &mut last), 0, "waits while busy");
        assert_eq!(tl.auto_key(&mut s, false, &mut last), 1);
        assert!(s.animation.track("exposure").is_some());
        assert_eq!(tl.auto_key(&mut s, false, &mut last), 0, "keyed once");
    }
    #[test]
    fn retiming_and_copying_keys() {
        let mut a = Animation::default();
        a.key("objects/0/size", 1., 1.);
        a.key("objects/0/size", 2., 2.);
        a.key("objects/1/size", 1., 5.);
        let only = vec!["objects/0".to_string()];
        assert_eq!(a.retime(&only, 1., 2., 0.5), 2);
        assert_eq!(a.times(&only), vec![1.5, 3.5]);
        assert_eq!(a.times(&["objects/1".to_string()]), vec![1.]);
        assert_eq!(a.copy_keys("objects/0", "objects/1"), 1);
        assert_eq!(a.track("objects/1/size").unwrap().keys.len(), 2);
    }
    #[test]
    fn the_flashlight_keys_nothing_rather_than_the_camera() {
        use crate::ui::Selection;
        let mut s = Settings::default();
        let mut tl = Timeline::default();
        tl.update(&mut s);
        assert_eq!(key_selection(&mut s, &tl, Selection::Flashlight), 0);
        assert!(s.animation.tracks.is_empty());
        assert!(prefixes(Selection::Flashlight).is_empty());
    }
    #[test]
    fn keying_a_selection_and_removing_an_object() {
        let mut s = Settings::default();
        let mut tl = Timeline::default();
        tl.update(&mut s);
        let n = key_selection(&mut s, &tl, crate::ui::Selection::Object(1));
        // position 2, rotation, tilt 2, size, stretch, scale_xz 2, elevation.
        assert_eq!(n, 10);
        tl.time = 1.;
        tl.update(&mut s);
        s.objects[1].material.color[0] = 0.2;
        s.exposure = 2.;
        assert_eq!(
            key_selection(&mut s, &tl, crate::ui::Selection::Object(1)),
            11
        );
        assert_eq!(key_changed(&mut s, &tl), 2);
        let camera = key_selection(&mut s, &tl, crate::ui::Selection::None);
        // yaw, pitch, distance, target ×3, fov, focus distance.
        assert_eq!(camera, 8);
        s.animation.key("objects/0/size", 0., 1.);
        s.animation.removed("objects", 0);
        assert!(
            s.animation
                .tracks
                .iter()
                .all(|t| !t.path.starts_with("objects/1"))
        );
        assert!(s.animation.track("objects/0/position/0").is_some());
        let o = ["objects/0".to_string()];
        assert_eq!(s.animation.times(&o), [0., 1.]);
        assert_eq!(s.animation.remove_keys(&o, 1.), 11);
        assert!(s.animation.track("objects/0/material/color/0").is_none());
    }
}
