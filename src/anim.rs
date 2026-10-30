//! Keyframe animation (roadmap track 8). A track drives one numeric field of `Settings`, named
//! by its JSON path ("yaw", "objects/2/position/0", "lights/0/color/1"), so anything stored as a
//! float can be animated without per-field code: camera, object transforms, lights, materials,
//! exposure. Integers, enums, strings and booleans are not animated. The animation lives in the
//! scene file (`Settings::animation`); the playhead lives in the app (`Timeline`).
use crate::scene::Settings;
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
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Track {
    pub path: String,
    /// Sorted by time, at least one.
    pub keys: Vec<Key>,
}
impl Track {
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
    pub fn insert(&mut self, time: f32, value: f64) {
        match self.keys.iter_mut().find(|k| (k.time - time).abs() < 1e-4) {
            Some(k) => k.value = value,
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
}
impl Default for Animation {
    fn default() -> Self {
        Self {
            length: 5.,
            fps: 30,
            shutter: 0.,
            tracks: vec![],
        }
    }
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
        self.tracks.is_empty()
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
        for t in &self.tracks {
            anyhow::ensure!(
                !t.keys.is_empty()
                    && t.path.len() <= 256
                    && t.keys
                        .iter()
                        .all(|k| k.time.is_finite() && k.value.is_finite())
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
        if self.tracks.is_empty() {
            return s.clone();
        }
        let Ok(mut v) = serde_json::to_value(s) else {
            return s.clone();
        };
        for track in self.tracks.iter().filter(|t| animatable(&t.path)) {
            if let Some(slot) = v.pointer_mut(&format!("/{}", track.path))
                && slot.is_f64()
                && let Some(n) = serde_json::Number::from_f64(track.value(t))
            {
                *slot = Value::Number(n);
            }
        }
        match serde_json::from_value::<Settings>(v) {
            Ok(mut out) => match out.validate() {
                Ok(()) => out,
                Err(_) => s.clone(),
            },
            Err(_) => s.clone(),
        }
    }
    /// Sets a key on `path` at `t` (replacing one at the same time).
    pub fn key(&mut self, path: &str, t: f32, value: f64) {
        match self.tracks.iter_mut().find(|tr| tr.path == path) {
            Some(tr) => tr.insert(t, value),
            None => self.tracks.push(Track {
                path: path.into(),
                keys: vec![Key {
                    time: t,
                    value,
                    interp: Interp::default(),
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
/// Every animatable float under `prefix` in `v` ("" = the whole settings), as (path, value).
pub fn leaves(v: &Value, prefix: &str) -> Vec<(String, f64)> {
    fn walk(v: &Value, path: String, out: &mut Vec<(String, f64)>) {
        match v {
            Value::Number(n) if n.is_f64() => {
                if animatable(&path) {
                    out.push((path, n.as_f64().unwrap_or(0.)));
                }
            }
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
    let mut out = vec![];
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
    /// The time and tracks `rest` was evaluated at; the animation is applied again when either
    /// changes (not for length, fps or shutter, which move no key).
    pub applied: Option<(f32, Vec<Track>)>,
}
impl Default for Timeline {
    fn default() -> Self {
        Self {
            time: 0.,
            playing: false,
            looping: true,
            rest: None,
            applied: None,
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
    /// Applies the animation when the playhead or the keys changed since the last time; edits
    /// at a still playhead stay until it moves (as in Blender), so they can be keyed.
    pub fn update(&mut self, s: &mut Settings) -> bool {
        // Without keys nothing is overridden, so nothing counts as an edit to key.
        if s.animation.is_empty() {
            self.rest = None;
        }
        let fresh = self
            .applied
            .as_ref()
            .is_none_or(|(t, a)| *t != self.time || *a != s.animation.tracks);
        if !fresh {
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
        self.applied = Some((self.time, s.animation.tracks.clone()));
        true
    }
    /// Takes `s` as the animation's current result without applying it (after keying, undo and
    /// redo, whose values must stay as they are).
    pub fn settle(&mut self, s: &Settings) {
        self.rest = (!s.animation.is_empty()).then(|| s.clone());
        self.applied = Some((self.time, s.animation.tracks.clone()));
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
    let candidates: Vec<(String, f64)> = if prefix.is_empty() {
        first.iter().flat_map(|f| leaves(&v, f)).collect()
    } else {
        leaves(&v, &prefix)
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
    for (path, value) in candidates {
        let changed = rest
            .pointer(&format!("/{path}"))
            .and_then(Value::as_f64)
            .is_some_and(|r| r != value);
        if s.animation.track(&path).is_some() || changed || (fresh && first_key(&path)) {
            s.animation.key(&path, tl.time, value);
            n += 1;
        }
    }
    n
}
/// Keys every float edited since the animation last applied, anywhere in the scene.
pub fn key_changed(s: &mut Settings, tl: &Timeline) -> usize {
    let v = serde_json::to_value(&*s).unwrap_or(Value::Null);
    let rest = match &tl.rest {
        Some(r) => serde_json::to_value(r).unwrap_or(Value::Null),
        None => v.clone(),
    };
    let mut n = 0;
    for (path, value) in leaves(&v, "") {
        let changed = rest
            .pointer(&format!("/{path}"))
            .and_then(Value::as_f64)
            .is_some_and(|r| r != value);
        if changed {
            s.animation.key(&path, tl.time, value);
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
                })
                .collect(),
        }
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
    fn apply_drives_floats_and_skips_the_rest() {
        let mut s = Settings::default();
        s.animation.key("objects/0/position/0", 0., -0.5);
        s.animation.key("objects/0/position/0", 2., 0.5);
        s.animation.tracks[0].keys[0].interp = Interp::Linear;
        s.animation.key("bounces", 0., 2.);
        s.animation.key("objects/9/size", 0., 2.);
        let out = s.animation.apply(&s, 1.);
        assert!(out.objects[0].position[0].abs() < 1e-6);
        assert_eq!(out.bounces, s.bounces);
        // Out-of-range values leave the settings alone.
        s.animation.key("exposure", 0., -1.);
        assert_eq!(
            s.animation.apply(&s, 1.).objects[0].position[0],
            s.objects[0].position[0]
        );
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
