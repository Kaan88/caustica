//! The command palette (Ctrl+K): find a setting or a hard-to-find command by name.
//!
//! A static table of entries, a small matcher and the palette's own state. Opening a setting
//! goes to its tab and selects what shows it (Stage, Camera, Main light, Flashlight, the first
//! object or area light); it does not scroll to the row, so the toast names the section.
//! Running an entry is `App::run_palette` (ui/windows.rs).
use crate::ui::Tab;

/// What a setting needs selected before its inspector shows (Lights, Object and Material).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Focus {
    /// Nothing: Render, Look and Export rows, or the outliner.
    Keep,
    Stage,
    Camera,
    MainLight,
    Flashlight,
    /// The selected object, else the first one.
    Object,
    /// The selected area light, else the first one.
    AreaLight,
}

/// Windows the palette opens. Serialized by name in the prefs (pinned top-bar buttons).
#[derive(Clone, Copy, PartialEq, Eq, Debug, serde::Serialize, serde::Deserialize)]
pub enum Win {
    Remote,
    Console,
    Stats,
    Shortcuts,
    Scenes,
    Assets,
    Video,
    Scopes,
    Graph,
    Paths,
    Describe,
    Puzzle,
}

/// Things the palette does. Serialized by name in the prefs (pinned top-bar buttons).
#[derive(Clone, Copy, PartialEq, Eq, Debug, serde::Serialize, serde::Deserialize)]
pub enum Do {
    OpenScene,
    SaveScene,
    SaveSceneAs,
    SaveSettings,
    Advanced,
    CpuView,
    Orbit,
    Walk,
    Flashlight,
    Denoiser,
    FreezeReference,
    Restart,
    FrameScene,
    ResetCamera,
    SavePng,
    HidePanel,
    UpdateCheck,
    LockExposure,
    /// Toggles the A/B split (freezing a reference first when there is none).
    Compare,
    Randomize,
    Today,
    HighRes,
}

/// Live controls that can be pinned to the top bar (Menu > Settings > Top bar).
#[derive(Clone, Copy, PartialEq, Eq, Debug, serde::Serialize, serde::Deserialize)]
pub enum Widget {
    /// The exposure in stops: the bias in auto, the exposure itself in manual.
    Exposure,
    Denoiser,
    MaxSamples,
}
impl Widget {
    pub const ALL: [Widget; 3] = [Widget::Exposure, Widget::Denoiser, Widget::MaxSamples];
    pub fn label(self) -> &'static str {
        match self {
            Widget::Exposure => "Exposure (EV)",
            Widget::Denoiser => "Denoiser",
            Widget::MaxSamples => "Sample cap",
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Act {
    /// A setting: its tab, what to select, and the section to name in the toast.
    Go(Tab, Focus, &'static str),
    Window(Win),
    Do(Do),
}

pub struct Entry {
    pub label: &'static str,
    /// Other words people search by.
    pub words: &'static str,
    /// The key, if it has one (shown at the right).
    pub key: &'static str,
    pub act: Act,
}

const fn go(
    label: &'static str,
    words: &'static str,
    tab: Tab,
    focus: Focus,
    section: &'static str,
) -> Entry {
    Entry {
        label,
        words,
        key: "",
        act: Act::Go(tab, focus, section),
    }
}
const fn win(label: &'static str, words: &'static str, key: &'static str, w: Win) -> Entry {
    Entry {
        label,
        words,
        key,
        act: Act::Window(w),
    }
}
const fn act(label: &'static str, words: &'static str, key: &'static str, d: Do) -> Entry {
    Entry {
        label,
        words,
        key,
        act: Act::Do(d),
    }
}

use Focus::*;
use Tab::{Export, Lights, Look, Material, Render};

/// Every entry. Settings first (one per section or well-known row), then the windows and
/// commands that sit in the Menu or behind a key. Always-visible buttons are left out.
pub const ENTRIES: &[Entry] = &[
    // Render tab
    go(
        "Renderer: GPU or CPU",
        "software cpu view",
        Render,
        Keep,
        "Renderer",
    ),
    go(
        "Denoiser",
        "rr ray reconstruction native svgf off raw",
        Render,
        Keep,
        "Reconstruction",
    ),
    go(
        "RR model",
        "ray reconstruction transformer preset",
        Render,
        Keep,
        "Reconstruction",
    ),
    go(
        "Denoise strength",
        "native denoiser filter",
        Render,
        Keep,
        "Reconstruction",
    ),
    go(
        "Quality mode",
        "dlss upscaler performance quality",
        Render,
        Keep,
        "Reconstruction",
    ),
    go(
        "Render scale",
        "resolution internal percent",
        Render,
        Keep,
        "Reconstruction",
    ),
    go("Upscaler", "dlss fsr xess", Render, Keep, "Reconstruction"),
    go(
        "Pixel filter",
        "antialiasing filter width",
        Render,
        Keep,
        "Reconstruction",
    ),
    go(
        "Samples per pixel",
        "spp samples cap",
        Render,
        Keep,
        "Sampling",
    ),
    go(
        "Sample pattern",
        "sobol blue noise white",
        Render,
        Keep,
        "Sampling",
    ),
    go(
        "Light sampling",
        "restir di candidates nee",
        Render,
        Keep,
        "Sampling",
    ),
    go(
        "Integrator",
        "path bdpt photon mapping bidirectional",
        Render,
        Keep,
        "Advanced",
    ),
    go(
        "Battle mode",
        "compare integrators quadrants",
        Render,
        Keep,
        "Advanced",
    ),
    go(
        "Spectral rendering",
        "dispersion wavelengths",
        Render,
        Keep,
        "Advanced",
    ),
    go(
        "Suppress fireflies",
        "clamp bright pixels",
        Render,
        Keep,
        "Advanced",
    ),
    go(
        "VSync",
        "swapchain present tearing",
        Render,
        Keep,
        "Advanced",
    ),
    // Look tab
    go(
        "Look (starting point)",
        "preset grade film",
        Look,
        Keep,
        "Image",
    ),
    go("Exposure", "brightness ev", Look, Keep, "Image"),
    go("Tone map", "tonemap aces agx filmic", Look, Keep, "Image"),
    go("Glare", "bloom spikes rings", Look, Keep, "Film"),
    go("Halation", "film glow", Look, Keep, "Film"),
    go("Film grain", "grain size noise", Look, Keep, "Film"),
    go("Film stock", "kodak portra velvia", Look, Keep, "Film"),
    go("Vignette", "corners darkening", Look, Keep, "Film"),
    go(
        "Lift, gamma, gain",
        "grade color wheels",
        Look,
        Keep,
        "Grade",
    ),
    go("LUT", "color lookup cube grade", Look, Keep, "Grade"),
    go(
        "HDR output",
        "hdr10 peak brightness paper white display",
        Look,
        Keep,
        "Display",
    ),
    go(
        "Compare settings",
        "a/b reference split freeze",
        Look,
        Keep,
        "Compare",
    ),
    go("Zebras", "overexposure stripes", Look, Keep, "Tools"),
    go(
        "Light path expressions",
        "lpe aov",
        Look,
        Keep,
        "Light path expressions",
    ),
    go(
        "Save the image",
        "png exr jpeg screenshot",
        Export,
        Keep,
        "Save the current image",
    ),
    go(
        "High-resolution still",
        "export render big",
        Export,
        Keep,
        "High-resolution still",
    ),
    go(
        "Video export",
        "turntable mp4 prores sequence",
        Export,
        Keep,
        "Video",
    ),
    // Lights, Object and Material tabs
    go(
        "Outliner",
        "objects list hierarchy",
        Tab::Object,
        Keep,
        "Outliner",
    ),
    go(
        "Room",
        "cornell walls floor ceiling",
        Tab::Object,
        Stage,
        "Room",
    ),
    go(
        "Wall material",
        "left right back wall floor",
        Material,
        Stage,
        "Walls and floor",
    ),
    go(
        "Sky light",
        "environment ambient sun physical sky",
        Lights,
        Keep,
        "Sky and sun",
    ),
    go(
        "HDRI",
        "environment panorama rotation strength",
        Lights,
        Keep,
        "HDRI",
    ),
    go("Fog", "volume smoke haze", Lights, Keep, "Fog"),
    go(
        "Camera",
        "orbit distance target field of view fov",
        Tab::Object,
        Camera,
        "Camera",
    ),
    go("Field of view", "fov angle", Tab::Object, Camera, "Camera"),
    go(
        "Lens",
        "aperture f-number depth of field focus bokeh",
        Tab::Object,
        Camera,
        "Lens",
    ),
    go(
        "Camera model",
        "orthographic panorama little planet fisheye",
        Tab::Object,
        Camera,
        "Lens",
    ),
    go(
        "Main light",
        "sun intensity size position",
        Lights,
        MainLight,
        "Main light",
    ),
    go(
        "Flashlight",
        "torch headlamp beam",
        Lights,
        Flashlight,
        "Flashlight",
    ),
    go(
        "Area light",
        "emitter panel disc sphere light",
        Lights,
        AreaLight,
        "Area light",
    ),
    go(
        "Audio: lights follow the music",
        "sound reactive bass beat",
        Lights,
        Keep,
        "Audio",
    ),
    go(
        "Object material",
        "glass metal roughness coating clear coat",
        Material,
        Object,
        "Material",
    ),
    go(
        "Object transform",
        "position rotation scale",
        Tab::Object,
        Object,
        "Object",
    ),
    go(
        "Physics",
        "simulate rigid bodies cloth gravity wind",
        Tab::Object,
        Keep,
        "Physics world",
    ),
    // Export tab
    go("Export", "output files", Export, Keep, "Export"),
    // Windows
    win("Phone remote", "qr web", "", Win::Remote),
    win("Script console", "rhai repl script", "", Win::Console),
    win("Statistics and achievements", "stats", "", Win::Stats),
    win("Keyboard shortcuts", "keys help", "F1", Win::Shortcuts),
    win(
        "All scenes",
        "browser open scene presets",
        "Ctrl+P",
        Win::Scenes,
    ),
    win(
        "Assets",
        "models textures hdris browser download",
        "",
        Win::Assets,
    ),
    win("Video export window", "render video", "", Win::Video),
    win("Scopes", "histogram waveform vectorscope", "", Win::Scopes),
    win("Convergence graph", "rmse noise reference", "", Win::Graph),
    win("Path visualizer", "trace pixel paths", "P", Win::Paths),
    win(
        "Describe a scene (AI)",
        "text to scene generate critique model",
        "",
        Win::Describe,
    ),
    win("Laser puzzle", "game mirrors beam levels", "", Win::Puzzle),
    // Commands
    act("Open scene", "load file", "Ctrl+O", Do::OpenScene),
    act("Save scene", "write", "Ctrl+S", Do::SaveScene),
    act(
        "Save scene as",
        "write copy",
        "Ctrl+Shift+S",
        Do::SaveSceneAs,
    ),
    act("Save settings.json", "write settings", "", Do::SaveSettings),
    act("Show advanced settings", "expert more", "", Do::Advanced),
    act("Render on the CPU", "software view", "", Do::CpuView),
    act("Auto orbit", "rotate turntable", "O", Do::Orbit),
    act("Walk mode", "wasd first person", "`", Do::Walk),
    act("Flashlight on or off", "torch", "T", Do::Flashlight),
    act("Cycle the denoiser", "rr native off", "Alt+D", Do::Denoiser),
    act(
        "Freeze the A/B reference",
        "compare",
        "C",
        Do::FreezeReference,
    ),
    act(
        "Restart the render",
        "reset samples accumulation",
        "R",
        Do::Restart,
    ),
    act("Frame the scene", "fit view", "F", Do::FrameScene),
    act("Reset the camera", "home", "Home", Do::ResetCamera),
    act("Save the image as PNG", "screenshot", "F12", Do::SavePng),
    act("Hide the panels", "screenshot mode", "Tab", Do::HidePanel),
    act(
        "Lock the exposure (AE-L)",
        "ae-l ael auto exposure manual freeze meter",
        "",
        Do::LockExposure,
    ),
    act(
        "Compare (A/B split)",
        "reference split before after",
        "",
        Do::Compare,
    ),
    act("Randomize the scene", "random variation", "", Do::Randomize),
    act("Seed of the day", "today random daily", "", Do::Today),
    act(
        "Render a high-resolution still",
        "export big image",
        "",
        Do::HighRes,
    ),
    act(
        "Check for updates at startup",
        "update",
        "",
        Do::UpdateCheck,
    ),
];

/// How well `query` matches `e`: lower is better, None is no match. Every word of the query
/// must be found: a prefix of the label beats a word start, which beats a substring of the label
/// or the search words, which beats letters in order (a typo-tolerant "dnoiser").
pub fn score(query: &str, e: &Entry) -> Option<u32> {
    let label = e.label.to_lowercase();
    let words = e.words.to_lowercase();
    let mut total = 0;
    for q in query.to_lowercase().split_whitespace() {
        total += if label.starts_with(q) {
            0
        } else if label
            .split(|c: char| !c.is_alphanumeric())
            .any(|w| w.starts_with(q))
        {
            1
        } else if label.contains(q) {
            2
        } else if words.split_whitespace().any(|w| w.starts_with(q)) {
            3
        } else if words.contains(q) {
            4
        } else if in_order(q, &label) {
            6
        } else {
            return None;
        };
    }
    Some(total)
}
fn in_order(q: &str, text: &str) -> bool {
    let mut chars = text.chars();
    q.chars().all(|c| chars.any(|t| t == c))
}
/// The entries to show, best first (the table's order breaks ties); an empty query lists the
/// commands and windows, which are what people forget.
pub fn matches(query: &str) -> Vec<usize> {
    let mut hits: Vec<(u32, usize)> = ENTRIES
        .iter()
        .enumerate()
        .filter_map(|(i, e)| score(query, e).map(|s| (s, i)))
        .collect();
    hits.sort();
    hits.into_iter().map(|(_, i)| i).collect()
}

#[derive(Default)]
pub struct State {
    pub open: bool,
    pub query: String,
    /// The highlighted row among the matches.
    pub at: usize,
    /// Drawn last frame: opening gives the box the keyboard.
    pub was_open: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn first(q: &str) -> Option<&'static str> {
        matches(q).first().map(|i| ENTRIES[*i].label)
    }

    #[test]
    fn finds_by_name_words_and_typos() {
        assert_eq!(first("denoiser"), Some("Denoiser"));
        assert_eq!(first("script"), Some("Script console"));
        assert_eq!(first("qr"), Some("Phone remote"));
        assert_eq!(first("dnoiser"), Some("Denoiser"));
        assert_eq!(first("bokeh"), Some("Lens"));
        assert!(matches("zzzzqq").is_empty());
    }

    #[test]
    fn every_word_must_match() {
        assert!(
            matches("lens aperture")
                .contains(&ENTRIES.iter().position(|e| e.label == "Lens").unwrap())
        );
        assert!(matches("lens zzzz").is_empty());
    }

    #[test]
    fn empty_query_lists_everything_in_table_order() {
        let all = matches("");
        assert_eq!(all.len(), ENTRIES.len());
        assert!(all.windows(2).all(|w| w[0] < w[1]));
    }

    #[test]
    fn labels_are_unique_and_keys_are_not_stale() {
        for (i, a) in ENTRIES.iter().enumerate() {
            assert!(
                ENTRIES[i + 1..].iter().all(|b| b.label != a.label),
                "{}",
                a.label
            );
        }
        let key = |d: Do| ENTRIES.iter().find(|e| e.act == Act::Do(d)).unwrap().key;
        assert_eq!(key(Do::Orbit), "O");
        assert_eq!(key(Do::Denoiser), "Alt+D");
    }
}
