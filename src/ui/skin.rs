//! The app's own look, "Instrument" (`docs/mockups/skin-b-instrument.dc.html` is the
//! specification; engine-notes "UI skin"). egui's painter has no gradient fill and no inner
//! shadow, so each control's chrome is drawn once on the CPU at the screen's pixels per point
//! (`raster`), kept in one atlas texture, and painted as a sliced mesh. The widgets here
//! allocate and paint themselves; the stock text widgets (a number being typed) run with their
//! own chrome switched off on top of a painted field.
//!
//! `scope` marks the properties panel, where a row's control fills the controls column; the
//! shared helpers in `ui.rs` (`row`, `fold`, `check_row`, the slider) draw these widgets.
use egui::{Color32, Pos2, Rect, Response, Sense, Shape, Ui, Vec2, pos2, vec2};
use std::cell::Cell;
use std::collections::HashMap;
use std::ops::RangeInclusive;
use std::sync::Arc;

pub(crate) const PANEL: Color32 = Color32::from_rgb(0x22, 0x22, 0x25);
const BAND: Color32 = Color32::from_rgb(0x2b, 0x2b, 0x2f);
const BAND_HOVER: Color32 = Color32::from_rgb(0x34, 0x34, 0x3a);
const BAND_PRESSED: Color32 = Color32::from_rgb(0x25, 0x25, 0x28);
const EDGE: Color32 = Color32::from_rgb(0x0c, 0x0c, 0x0d);
const LIGHT_LINE: Color32 = Color32::from_rgb(0x38, 0x38, 0x3d);
pub(crate) const TX: Color32 = Color32::from_rgb(0xe5, 0xe5, 0xe7);
pub(crate) const DIM: Color32 = Color32::from_rgb(0xa8, 0xa8, 0xaf);
pub(crate) const FAINT: Color32 = Color32::from_rgb(0x8e, 0x8e, 0x96);
/// Text on the accent.
const INK: Color32 = Color32::from_rgb(0x06, 0x27, 0x2c);
const ACC: Color32 = super::ACCENT;

/// A control's height, and a row's (the control plus 4 points).
pub(crate) const CONTROL: f32 = 24.;
/// The least width of a number field; a long value widens it.
const NUMBER: f32 = 58.;

// ---------------------------------------------------------------------------------------------
// Fonts.

const SANS: &str = "plex";
const SEMIBOLD: &str = "plex semibold";
const MONO: &str = "plex mono";
/// IBM Plex as three named families, each falling back to egui's own fonts (and Hack, which
/// `theme` appends) for the glyphs Plex lacks. Call after the stock families are final.
pub(crate) fn fonts(defs: &mut egui::FontDefinitions) {
    use egui::FontFamily;
    for (name, bytes) in [
        (
            "IBM Plex Sans",
            &include_bytes!("../../assets/fonts/IBMPlexSans-Regular.ttf")[..],
        ),
        (
            "IBM Plex Sans SemiBold",
            &include_bytes!("../../assets/fonts/IBMPlexSans-SemiBold.ttf")[..],
        ),
        (
            "IBM Plex Mono",
            &include_bytes!("../../assets/fonts/IBMPlexMono-Regular.ttf")[..],
        ),
    ] {
        defs.font_data.insert(
            name.to_owned(),
            Arc::new(egui::FontData::from_static(bytes)),
        );
    }
    let stock = |defs: &egui::FontDefinitions, family: FontFamily| {
        defs.families.get(&family).cloned().unwrap_or_default()
    };
    let sans = stock(defs, FontFamily::Proportional);
    let mut mono = stock(defs, FontFamily::Monospace);
    let more: Vec<String> = sans.iter().filter(|f| !mono.contains(f)).cloned().collect();
    mono.extend(more);
    // Plex leads the stock families too, so everything egui draws itself is in it.
    for (family, font) in [
        (FontFamily::Proportional, "IBM Plex Sans"),
        (FontFamily::Monospace, "IBM Plex Mono"),
    ] {
        defs.families
            .entry(family)
            .or_default()
            .insert(0, font.to_owned());
    }
    for (family, font, rest) in [
        (SANS, "IBM Plex Sans", &sans),
        (SEMIBOLD, "IBM Plex Sans SemiBold", &sans),
        (MONO, "IBM Plex Mono", &mono),
    ] {
        let mut list = vec![font.to_owned()];
        list.extend(rest.iter().cloned());
        defs.families.insert(FontFamily::Name(family.into()), list);
    }
}
pub(crate) fn sans(size: f32) -> egui::FontId {
    egui::FontId::new(size, egui::FontFamily::Name(SANS.into()))
}
pub(crate) fn semibold(size: f32) -> egui::FontId {
    egui::FontId::new(size, egui::FontFamily::Name(SEMIBOLD.into()))
}
pub(crate) fn mono(size: f32) -> egui::FontId {
    egui::FontId::new(size, egui::FontFamily::Name(MONO.into()))
}

// ---------------------------------------------------------------------------------------------
// The scope a tab draws the new look in.

thread_local! {
    static DEPTH: Cell<u32> = const { Cell::new(0) };
    static BLEED: Cell<f32> = const { Cell::new(0.) };
    static LINEAR: Cell<bool> = const { Cell::new(false) };
    /// The state the sheet shows a control in, whatever the pointer does.
    static FORCED: Cell<Option<Forced>> = const { Cell::new(None) };
}
#[derive(Clone, Copy, Default)]
struct Forced {
    hover: bool,
    press: bool,
    focus: bool,
}
/// Draws `add` with its controls held in `state` (the sheet's columns).
fn forced<R>(state: Forced, add: impl FnOnce() -> R) -> R {
    let before = FORCED.replace(Some(state));
    let r = add();
    FORCED.set(before);
    r
}
/// A control has the keyboard focus (or the sheet says so).
fn focused(r: &Response) -> bool {
    FORCED.get().map_or_else(|| r.has_focus(), |f| f.focus)
}
/// How the renderer blends the UI this frame: in linear light on an scRGB HDR swap chain
/// (gui.hlsl), on sRGB code values otherwise. The board is specified as CSS, which blends code
/// values, so in linear light the translucent pixels are drawn with a corrected alpha.
pub(crate) fn blend_in_linear_light(linear: bool) {
    LINEAR.set(linear);
}
/// True inside `scope` (the properties panel): a row's control fills the controls column.
pub(crate) fn fills() -> bool {
    DEPTH.get() > 0
}
/// Draws `add` in the new look. `bleed` is how far the section bands reach past the Ui's sides
/// (the panel's own padding), so they run from edge to edge.
pub(crate) fn scope<R>(ui: &mut Ui, bleed: f32, add: impl FnOnce(&mut Ui) -> R) -> R {
    struct Leave(f32);
    impl Drop for Leave {
        fn drop(&mut self) {
            DEPTH.set(DEPTH.get() - 1);
            BLEED.set(self.0);
        }
    }
    let _leave = Leave(BLEED.replace(bleed));
    DEPTH.set(DEPTH.get() + 1);
    ui.scope(|ui| {
        // Sections stack with no gap; a section's body sets its own row gap.
        ui.spacing_mut().item_spacing = vec2(8., 0.);
        add(ui)
    })
    .inner
}
/// The style of what egui still draws itself inside the scope: labels, notes, a field's text.
pub(super) fn style(s: &mut egui::Style) {
    use egui::TextStyle;
    s.text_styles = [
        (TextStyle::Heading, semibold(14.)),
        (TextStyle::Body, sans(12.5)),
        (TextStyle::Button, sans(12.5)),
        (TextStyle::Small, sans(11.5)),
        (TextStyle::Monospace, mono(11.5)),
    ]
    .into();
    s.drag_value_text_style = TextStyle::Monospace;
    s.spacing.item_spacing = vec2(8., 6.);
    s.spacing.interact_size = vec2(40., CONTROL);
    s.spacing.button_padding = vec2(9., 3.);
    let v = &mut s.visuals;
    v.disabled_alpha = 0.42;
    v.selection.bg_fill = ACC.gamma_multiply(0.45);
    v.selection.stroke = egui::Stroke::new(1_f32, TX);
    v.hyperlink_color = ACC;
    // What egui still draws itself (a radio, a text field, a menu-bar button, a window) in
    // the board's flat colours: the middle of a gradient, the border, the recess.
    v.panel_fill = PANEL;
    // egui builds a tooltip (and any popup nobody gave a frame) from these: the board's `.tip`.
    // Menus, combos, windows and cards paint their own chrome and never read them.
    v.window_fill = Color32::from_rgb(0x0f, 0x0f, 0x10);
    v.window_stroke = egui::Stroke::new(1_f32, Color32::from_rgb(0x3a, 0x3a, 0x3f));
    v.window_corner_radius = 6.into();
    v.menu_corner_radius = 3.into();
    v.popup_shadow = egui::Shadow {
        offset: [0, 6],
        blur: 16,
        spread: 0,
        color: Color32::from_black_alpha(153),
    };
    s.spacing.menu_margin = egui::Margin::symmetric(8, 4);
    let v = &mut s.visuals;
    v.extreme_bg_color = Color32::from_rgb(0x14, 0x14, 0x16);
    v.faint_bg_color = BAND;
    let radius = egui::CornerRadius::same(4);
    let edge = egui::Stroke::new(1_f32, EDGE);
    let w = &mut v.widgets;
    w.noninteractive.fg_stroke.color = TX;
    w.noninteractive.bg_stroke = egui::Stroke::new(1_f32, LIGHT_LINE);
    w.noninteractive.bg_fill = PANEL;
    w.noninteractive.corner_radius = radius;
    for (state, fill, text) in [
        (&mut w.inactive, Color32::from_rgb(0x3a, 0x3a, 0x3f), TX),
        (
            &mut w.hovered,
            Color32::from_rgb(0x44, 0x44, 0x4a),
            Color32::WHITE,
        ),
        (
            &mut w.active,
            Color32::from_rgb(0x2c, 0x2c, 0x30),
            Color32::WHITE,
        ),
        (&mut w.open, Color32::from_rgb(0x2c, 0x2c, 0x30), TX),
    ] {
        state.bg_fill = fill;
        state.weak_bg_fill = fill;
        state.bg_stroke = edge;
        state.fg_stroke.color = text;
        state.corner_radius = radius;
        state.expansion = 0.;
    }
}
/// Switches off the chrome egui draws for a button or a text field, leaving its text: the
/// field behind it is painted here.
fn bare(s: &mut egui::Style) {
    let v = &mut s.visuals;
    for w in [
        &mut v.widgets.inactive,
        &mut v.widgets.hovered,
        &mut v.widgets.active,
        &mut v.widgets.open,
    ] {
        w.bg_fill = Color32::TRANSPARENT;
        w.weak_bg_fill = Color32::TRANSPARENT;
        w.bg_stroke = egui::Stroke::NONE;
        w.fg_stroke.color = TX;
        w.expansion = 0.;
    }
    v.extreme_bg_color = Color32::TRANSPARENT;
    v.text_edit_bg_color = Some(Color32::TRANSPARENT);
    v.selection.stroke = egui::Stroke::NONE;
    v.text_cursor.stroke.color = TX;
}

// ---------------------------------------------------------------------------------------------
// The rasteriser: one control's chrome as a CSS box (rounded border box, vertical gradient,
// inset and outer shadows), at the screen's pixels per point.

/// One `box-shadow`: offset down, blur radius, spread, straight RGBA in 0..1.
#[derive(Clone, Copy)]
struct Shadow {
    dy: f32,
    blur: f32,
    spread: f32,
    color: [f32; 4],
}
const fn shadow(dy: f32, blur: f32, spread: f32, rgb: Color32, alpha: f32) -> Shadow {
    Shadow {
        dy,
        blur,
        spread,
        color: [
            rgb.r() as f32 / 255.,
            rgb.g() as f32 / 255.,
            rgb.b() as f32 / 255.,
            alpha,
        ],
    }
}
const fn black(dy: f32, blur: f32, alpha: f32) -> Shadow {
    shadow(dy, blur, 0., Color32::BLACK, alpha)
}
/// A one-point line of white: under the top edge as an inset shadow (the highlight), under
/// the bottom edge as an outer one.
const fn white(alpha: f32) -> Shadow {
    shadow(1., 0., 0., Color32::WHITE, alpha)
}
/// A box in points; the lengths are scaled by the pixels per point when it is drawn.
struct Spec {
    /// Corner radii: top left, top right, bottom right, bottom left.
    radius: [f32; 4],
    /// A one-point border.
    border: Option<Color32>,
    /// The fill's top and bottom colours.
    fill: (Color32, Color32),
    /// CSS `filter: brightness()`.
    gain: f32,
    /// The outer shadows start this far outside the box (an outline's offset).
    hole: f32,
    /// First is on top, as in CSS.
    inset: Vec<Shadow>,
    outer: Vec<Shadow>,
}
impl Spec {
    /// How far the outer shadows reach past the box, in points.
    fn pad(&self) -> f32 {
        self.outer
            .iter()
            .map(|s| s.dy.abs() + s.spread + s.blur * 1.5)
            .fold(0., f32::max)
            .ceil()
    }
    /// How far in from an edge the box still differs from its middle, in points.
    fn cap(&self) -> f32 {
        let radius = self.radius.iter().copied().fold(0., f32::max);
        let inset = self
            .inset
            .iter()
            .map(|s| s.dy.abs() + s.spread + s.blur * 1.5)
            .fold(1., f32::max);
        (radius + inset).ceil() + 1.
    }
}
/// Signed distance to a rounded box centred on the origin (negative inside).
fn distance(p: Vec2, half: Vec2, radius: [f32; 4]) -> f32 {
    let r = match (p.x > 0., p.y > 0.) {
        (false, false) => radius[0],
        (true, false) => radius[1],
        (true, true) => radius[2],
        (false, true) => radius[3],
    }
    .min(half.x)
    .min(half.y)
    .max(0.);
    let q = vec2(p.x.abs() - half.x + r, p.y.abs() - half.y + r);
    q.x.max(q.y).min(0.) + vec2(q.x.max(0.), q.y.max(0.)).length() - r
}
/// Abramowitz and Stegun 7.1.26, good to 1.5e-7.
fn erfc(x: f32) -> f32 {
    let z = x.abs();
    let t = 1. / (1. + 0.327_591_1 * z);
    let poly = t
        * (0.254_829_6
            + t * (-0.284_496_74 + t * (1.421_413_7 + t * (-1.453_152 + t * 1.061_405_4))));
    let y = poly * (-z * z).exp();
    if x >= 0. { y } else { 2. - y }
}
/// How much of a pixel `d` pixels from an edge the shape covers: a box filter for a sharp
/// edge, a Gaussian of standard deviation blur / 2 (as CSS defines the blur radius) otherwise.
fn coverage(d: f32, blur: f32) -> f32 {
    if blur <= 0. {
        (0.5 - d).clamp(0., 1.)
    } else {
        0.5 * erfc(d / (blur * 0.5 * std::f32::consts::SQRT_2))
    }
}
/// `top` over `under`, both premultiplied.
fn over(top: [f32; 4], under: [f32; 4]) -> [f32; 4] {
    let k = 1. - top[3];
    [
        top[0] + under[0] * k,
        top[1] + under[1] * k,
        top[2] + under[2] * k,
        top[3] + under[3] * k,
    ]
}
fn premultiplied(c: [f32; 4]) -> [f32; 4] {
    [c[0] * c[3], c[1] * c[3], c[2] * c[3], c[3]]
}
fn rgba(c: Color32) -> [f32; 4] {
    // The specification's fills are opaque or plain transparent, so unmultiplying is exact.
    let [r, g, b, a] = c.to_srgba_unmultiplied();
    [
        r as f32 / 255.,
        g as f32 / 255.,
        b as f32 / 255.,
        a as f32 / 255.,
    ]
}
fn to_linear(c: f32) -> f32 {
    if c <= 0.04045 {
        c / 12.92
    } else {
        ((c + 0.055) / 1.055).powf(2.4)
    }
}
/// A translucent premultiplied pixel with its alpha changed so that blending it in linear
/// light over the panel gives what blending the original on code values would: a drop
/// shadow keeps its weight and a faint light line stays faint. Over other backdrops (a popup
/// over the image) it is the nearest there is without reading what is behind.
fn for_linear_blend(c: [f32; 4]) -> [f32; 4] {
    let a = c[3];
    if !(0.002..0.998).contains(&a) {
        return c;
    }
    let backdrop = rgba(PANEL);
    let (mut num, mut den) = (0., 0.);
    let mut straight = [0.; 3];
    for k in 0..3 {
        straight[k] = (c[k] / a).clamp(0., 1.);
        let wanted = to_linear(c[k] + backdrop[k] * (1. - a));
        let (from, to) = (to_linear(backdrop[k]), to_linear(straight[k]));
        num += (wanted - from) * (to - from);
        den += (to - from) * (to - from);
    }
    if den < 1e-9 {
        return c;
    }
    let a = (num / den).clamp(0., 1.);
    [straight[0] * a, straight[1] * a, straight[2] * a, a]
}
/// Draws `spec` as a box of `w` × `h` pixels with `pad` pixels around it for the outer
/// shadows. Colours are blended in gamma space, as CSS and egui both do.
fn raster(spec: &Spec, w: usize, h: usize, pad: usize, ppp: f32, linear: bool) -> Vec<Color32> {
    let (width, height) = (w + 2 * pad, h + 2 * pad);
    let half = vec2(w as f32, h as f32) / 2.;
    let radius = spec.radius.map(|r| r * ppp);
    let border = if spec.border.is_some() { ppp } else { 0. };
    let inner_half = half - Vec2::splat(border);
    let inner_radius = radius.map(|r| (r - border).max(0.));
    let grow =
        |radius: [f32; 4], by: f32| radius.map(|r| if r > 0. { (r + by).max(0.) } else { 0. });
    let (top, bottom) = (rgba(spec.fill.0), rgba(spec.fill.1));
    let mut pixels = Vec::with_capacity(width * height);
    for py in 0..height {
        for px in 0..width {
            let at = vec2(px as f32 + 0.5 - pad as f32, py as f32 + 0.5 - pad as f32);
            let p = at - half;
            let in_box = coverage(distance(p, half, radius), 0.);
            let in_fill = coverage(distance(p, inner_half, inner_radius), 0.);
            let mut out = [0.; 4];
            for s in spec.outer.iter().rev() {
                let by = s.spread * ppp;
                let d = distance(
                    p - vec2(0., s.dy * ppp),
                    half + Vec2::splat(by),
                    grow(radius, by),
                );
                let a = s.color[3] * coverage(d, s.blur * ppp);
                out = over(premultiplied([s.color[0], s.color[1], s.color[2], a]), out);
            }
            let outside = if spec.hole > 0. {
                let by = spec.hole * ppp;
                let d = distance(p, half + Vec2::splat(by), grow(radius, by));
                1. - coverage(d, 0.)
            } else {
                1. - in_box
            };
            out = out.map(|v| v * outside);
            let t = ((at.y - border) / (h as f32 - 2. * border).max(1.)).clamp(0., 1.);
            let mut fill = premultiplied([
                top[0] + (bottom[0] - top[0]) * t,
                top[1] + (bottom[1] - top[1]) * t,
                top[2] + (bottom[2] - top[2]) * t,
                top[3] + (bottom[3] - top[3]) * t,
            ]);
            for s in spec.inset.iter().rev() {
                let by = s.spread * ppp;
                let d = distance(
                    p - vec2(0., s.dy * ppp),
                    inner_half - Vec2::splat(by),
                    grow(inner_radius, -by),
                );
                let a = s.color[3] * (1. - coverage(d, s.blur * ppp));
                fill = over(premultiplied([s.color[0], s.color[1], s.color[2], a]), fill);
            }
            let mut body = match spec.border {
                Some(c) => {
                    let c = rgba(c);
                    [
                        c[0] + (fill[0] - c[0]) * in_fill,
                        c[1] + (fill[1] - c[1]) * in_fill,
                        c[2] + (fill[2] - c[2]) * in_fill,
                        1. + (fill[3] - 1.) * in_fill,
                    ]
                }
                None => fill,
            };
            let alpha = body[3];
            for v in &mut body[..3] {
                *v = (*v * spec.gain).min(alpha);
            }
            let mut c = [
                out[0] + body[0] * in_box,
                out[1] + body[1] * in_box,
                out[2] + body[2] * in_box,
                out[3] + body[3] * in_box,
            ];
            if linear {
                c = for_linear_blend(c);
            }
            let c = c.map(|v| (v.clamp(0., 1.) * 255. + 0.5) as u8);
            pixels.push(Color32::from_rgba_premultiplied(c[0], c[1], c[2], c[3]));
        }
    }
    pixels
}

// ---------------------------------------------------------------------------------------------
// The looks, from the board's style sheet.

/// Where a button sits in a segment: the neighbours share a border and only the ends are
/// rounded.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum Seg {
    Solo,
    First,
    Middle,
    Last,
}
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
enum Look {
    Raised,
    RaisedHover,
    Pressed,
    On,
    OnHover,
    OnPressed,
    /// A flat button under the pointer: raised, without the drop shadow.
    FlatHover,
    Primary,
    PrimaryHover,
    PrimaryPressed,
    /// The drop shadow under a whole segment.
    SegmentShadow,
    Field,
    FieldHover,
    FieldFocus,
    Groove,
    Thumb,
    ThumbHover,
    ThumbPressed,
    Check,
    CheckHover,
    CheckOn,
    CheckOnHover,
    Radio,
    RadioHover,
    RadioOn,
    RadioOnHover,
    Track,
    TrackOn,
    Knob,
    KnobHover,
    KnobPressed,
    Popup,
    /// Focus rings: an accent line outside a control of this corner radius.
    Ring(u8),
    /// The primary button's: a gap in the panel colour first.
    RingPrimary,
    /// The checkbox's and the switch's: an outline 1.5 points off the control.
    Outline(u8),
    /// A browser card (a scene, an asset): a band-coloured tile.
    Card,
    CardHover,
}
/// Which sides of a look stretch: neither (a thumb), the width (a button, whose gradient
/// fixes the height), or both (a popup, whose fill is flat).
#[derive(Clone, Copy, PartialEq)]
enum Stretch {
    None,
    Wide,
    Both,
}
impl Look {
    fn stretch(self) -> Stretch {
        match self {
            Look::Thumb
            | Look::ThumbHover
            | Look::ThumbPressed
            | Look::Check
            | Look::CheckHover
            | Look::CheckOn
            | Look::CheckOnHover
            | Look::Radio
            | Look::RadioHover
            | Look::RadioOn
            | Look::RadioOnHover
            | Look::Track
            | Look::TrackOn
            | Look::Knob
            | Look::KnobHover
            | Look::KnobPressed => Stretch::None,
            Look::Popup
            | Look::Card
            | Look::CardHover
            | Look::Ring(_)
            | Look::RingPrimary
            | Look::Outline(_) => Stretch::Both,
            _ => Stretch::Wide,
        }
    }
}
const fn mix(a: Color32, b: Color32, t: f32) -> Color32 {
    const fn channel(a: u8, b: u8, t: f32) -> u8 {
        (a as f32 + (b as f32 - a as f32) * t + 0.5) as u8
    }
    Color32::from_rgb(
        channel(a.r(), b.r(), t),
        channel(a.g(), b.g(), t),
        channel(a.b(), b.b(), t),
    )
}
fn spec(look: Look, seg: Seg) -> Spec {
    let rgb = Color32::from_rgb;
    let flat = |c: Color32| (c, c);
    let raised = (rgb(0x3b, 0x3c, 0x41), rgb(0x2f, 0x30, 0x34));
    let on = (rgb(0x1b, 0x1c, 0x1e), rgb(0x22, 0x23, 0x26));
    let accent = (mix(ACC, Color32::WHITE, 0.14), ACC);
    let metal = (rgb(0xe2, 0xe2, 0xe6), rgb(0xa6, 0xa6, 0xad));
    let metal_pressed = (rgb(0xa6, 0xa6, 0xad), rgb(0xc9, 0xc9, 0xce));
    let field = rgb(0x14, 0x14, 0x16);
    let well = rgb(0x12, 0x12, 0x14);
    let highlight = white(0.08);
    let solo = seg == Seg::Solo;
    let mut s = Spec {
        radius: match seg {
            Seg::Solo => [4.; 4],
            Seg::First => [4., 0., 0., 4.],
            Seg::Middle => [0.; 4],
            Seg::Last => [0., 4., 4., 0.],
        },
        border: Some(EDGE),
        fill: raised,
        gain: 1.,
        hole: 0.,
        inset: vec![],
        outer: vec![],
    };
    // A segment's drop shadow is one shadow under the group (`SegmentShadow`).
    let drop = |alpha: f32| {
        if solo {
            vec![black(1., 1., alpha)]
        } else {
            vec![]
        }
    };
    match look {
        Look::Raised | Look::RaisedHover | Look::FlatHover => {
            if look == Look::RaisedHover {
                s.fill = (rgb(0x46, 0x47, 0x4d), rgb(0x38, 0x39, 0x3e));
            }
            s.inset = vec![highlight];
            if look != Look::FlatHover {
                s.outer = drop(0.3);
            }
        }
        Look::Pressed => {
            s.fill = (rgb(0x27, 0x28, 0x2b), rgb(0x2d, 0x2e, 0x32));
            s.inset = vec![black(1., 3., 0.6)];
        }
        Look::On | Look::OnHover | Look::OnPressed => {
            s.fill = on;
            s.inset = vec![black(1., 3., 0.7)];
            if solo {
                s.outer = vec![white(0.05)];
            }
            s.gain = match look {
                Look::OnHover => 1.2,
                Look::OnPressed => 0.85,
                _ => 1.,
            };
        }
        Look::Primary | Look::PrimaryHover => {
            s.fill = accent;
            s.inset = vec![white(0.35)];
            s.outer = drop(0.3);
            s.gain = if look == Look::PrimaryHover { 1.08 } else { 1. };
        }
        Look::PrimaryPressed => {
            s.fill = accent;
            s.inset = vec![black(1., 3., 0.4)];
            s.gain = 0.88;
        }
        Look::SegmentShadow => {
            s.border = None;
            s.fill = flat(Color32::TRANSPARENT);
            s.outer = vec![black(1., 1., 0.3)];
        }
        Look::Field | Look::FieldHover | Look::FieldFocus => {
            s.fill = flat(if look == Look::FieldHover {
                rgb(0x19, 0x19, 0x1c)
            } else {
                field
            });
            s.inset = vec![black(1., 2., 0.6)];
            if look == Look::FieldFocus {
                s.border = Some(ACC);
                s.outer = vec![shadow(0., 0., 1., ACC, 1.)];
            } else {
                s.outer = vec![white(0.05)];
            }
        }
        Look::Groove => {
            s.radius = [3.; 4];
            s.border = None;
            s.fill = flat(well);
            s.inset = vec![black(1., 2., 0.8)];
            s.outer = vec![white(0.06)];
        }
        Look::Thumb | Look::ThumbHover | Look::ThumbPressed => {
            s.radius = [3.; 4];
            s.fill = if look == Look::ThumbPressed {
                metal_pressed
            } else {
                metal
            };
            s.inset = vec![white(0.6)];
            s.outer = vec![black(1., 2., 0.5)];
            s.gain = if look == Look::ThumbHover { 1.12 } else { 1. };
        }
        Look::Check | Look::CheckHover => {
            s.radius = [3.; 4];
            s.fill = flat(if look == Look::CheckHover {
                rgb(0x1d, 0x1d, 0x20)
            } else {
                field
            });
            s.inset = vec![black(1., 2., 0.7)];
            s.outer = vec![white(0.07)];
        }
        Look::CheckOn | Look::CheckOnHover => {
            s.radius = [3.; 4];
            s.fill = accent;
            s.inset = vec![white(0.35)];
            s.gain = if look == Look::CheckOnHover { 1.1 } else { 1. };
        }
        // The checkbox, round.
        Look::Radio | Look::RadioHover => {
            s.radius = [8.; 4];
            s.fill = flat(if look == Look::RadioHover {
                rgb(0x1d, 0x1d, 0x20)
            } else {
                field
            });
            s.inset = vec![black(1., 2., 0.7)];
            s.outer = vec![white(0.07)];
        }
        Look::RadioOn | Look::RadioOnHover => {
            s.radius = [8.; 4];
            s.fill = accent;
            s.inset = vec![white(0.35)];
            s.gain = if look == Look::RadioOnHover { 1.1 } else { 1. };
        }
        Look::Track | Look::TrackOn => {
            s.radius = [8.; 4];
            s.fill = flat(if look == Look::TrackOn {
                mix(ACC, Color32::BLACK, 0.3)
            } else {
                well
            });
            s.inset = vec![black(1., 2., 0.8)];
            s.outer = vec![white(0.06)];
        }
        Look::Knob | Look::KnobHover | Look::KnobPressed => {
            s.radius = [6.; 4];
            s.border = None;
            s.fill = if look == Look::KnobPressed {
                metal_pressed
            } else {
                metal
            };
            s.inset = vec![white(0.6)];
            s.outer = vec![black(1., 2., 0.5)];
            s.gain = if look == Look::KnobHover { 1.12 } else { 1. };
        }
        Look::Popup => {
            s.radius = [5.; 4];
            s.fill = flat(rgb(0x2a, 0x2a, 0x2e));
            s.inset = vec![highlight];
            s.outer = vec![black(14., 36., 0.7)];
        }
        Look::Card | Look::CardHover => {
            s.radius = [5.; 4];
            s.fill = flat(if look == Look::CardHover {
                BAND_HOVER
            } else {
                BAND
            });
            s.inset = vec![highlight];
            s.outer = vec![black(1., 2., 0.35)];
        }
        Look::Ring(radius) => {
            s.radius = [radius as f32; 4];
            s.border = None;
            s.fill = flat(Color32::TRANSPARENT);
            s.outer = vec![shadow(0., 0., 1.5, ACC, 1.)];
        }
        Look::RingPrimary => {
            s.border = None;
            s.fill = flat(Color32::TRANSPARENT);
            s.outer = vec![shadow(0., 0., 2., PANEL, 1.), shadow(0., 0., 3.5, ACC, 1.)];
        }
        Look::Outline(radius) => {
            s.radius = [radius as f32; 4];
            s.border = None;
            s.fill = flat(Color32::TRANSPARENT);
            s.outer = vec![shadow(0., 0., 3., ACC, 1.)];
            s.hole = 1.5;
        }
    }
    s
}

// ---------------------------------------------------------------------------------------------
// The atlas: every look drawn so far, in one texture (the renderers keep few egui textures and
// upload each whole).

type Key = (Look, Seg, u16, u16);
#[derive(Clone, Copy)]
struct Entry {
    /// The corner in the atlas and the size, in pixels.
    at: [usize; 2],
    size: [usize; 2],
    /// Pixels around the box for its outer shadows.
    pad: usize,
    /// The unstretched ends, pad included; 0 when that side does not stretch.
    cap: [usize; 2],
}
struct Atlas {
    ppp: f32,
    /// Drawn for blending in linear light (`blend_in_linear_light`).
    linear: bool,
    side: usize,
    pixels: Vec<Color32>,
    /// The shelf being filled: the next free corner and the shelf's height.
    cursor: [usize; 3],
    entries: HashMap<Key, Entry>,
    dirty: bool,
    /// A look did not fit: `flush` starts a larger atlas.
    full: bool,
    /// Start over one size up at the next frame's first look.
    grow: bool,
    texture: Option<egui::TextureHandle>,
}
impl Atlas {
    fn new() -> Self {
        Self {
            ppp: 0.,
            linear: false,
            side: 512,
            pixels: vec![],
            cursor: [1; 3],
            entries: HashMap::new(),
            dirty: false,
            full: false,
            grow: false,
            texture: None,
        }
    }
    fn clear(&mut self) {
        self.pixels = vec![Color32::TRANSPARENT; self.side * self.side];
        self.cursor = [1, 1, 0];
        self.entries.clear();
        self.dirty = true;
        self.full = false;
    }
    fn entry(&mut self, key: Key, ppp: f32, linear: bool) -> Option<Entry> {
        if self.grow {
            self.grow = false;
            self.side = (self.side * 2).min(4096);
            self.pixels.clear();
        }
        if self.ppp != ppp || self.linear != linear || self.pixels.is_empty() {
            self.ppp = ppp;
            self.linear = linear;
            self.clear();
        }
        if let Some(e) = self.entries.get(&key) {
            return Some(*e);
        }
        if self.full {
            return None;
        }
        let (look, seg, w, h) = key;
        let spec = spec(look, seg);
        let px = |points: f32| (points * ppp).ceil() as usize;
        let pad = px(spec.pad());
        let cap = px(spec.cap());
        let stretch = look.stretch();
        let (w, cap_x) = match stretch {
            Stretch::None => (w as usize, 0),
            _ => (2 * cap + 1, pad + cap),
        };
        let (h, cap_y) = match stretch {
            Stretch::Both => (2 * cap + 1, pad + cap),
            _ => (h as usize, 0),
        };
        let size = [w + 2 * pad, h + 2 * pad];
        if self.cursor[0] + size[0] + 1 > self.side {
            self.cursor = [1, self.cursor[1] + self.cursor[2] + 1, 0];
        }
        if size[0] + 2 > self.side || self.cursor[1] + size[1] + 1 > self.side {
            self.full = true;
            return None;
        }
        let at = [self.cursor[0], self.cursor[1]];
        let image = raster(&spec, w, h, pad, ppp, linear);
        for (row, line) in image.chunks_exact(size[0]).enumerate() {
            let start = (at[1] + row) * self.side + at[0];
            self.pixels[start..start + size[0]].copy_from_slice(line);
        }
        self.cursor[0] += size[0] + 1;
        self.cursor[2] = self.cursor[2].max(size[1]);
        self.dirty = true;
        let entry = Entry {
            at,
            size,
            pad,
            cap: [cap_x, cap_y],
        };
        self.entries.insert(key, entry);
        Some(entry)
    }
}
fn atlas(ctx: &egui::Context) -> Arc<egui::mutex::Mutex<Atlas>> {
    ctx.data_mut(|d| {
        d.get_temp_mut_or_insert_with(egui::Id::unique("skin atlas"), || {
            Arc::new(egui::mutex::Mutex::new(Atlas::new()))
        })
        .clone()
    })
}
/// Uploads what this frame's UI drew into the atlas. Call once at the end of the UI pass.
pub(crate) fn flush(ctx: &egui::Context) {
    let atlas = atlas(ctx);
    let mut a = atlas.lock();
    if a.full && !a.grow && a.side < 4096 {
        // The next frame starts over one size up (`entry`): this frame's meshes point into
        // the atlas as it is, so it stays; the looks that did not fit are drawn then.
        a.grow = true;
        ctx.request_repaint();
    }
    if !a.dirty || a.pixels.is_empty() {
        return;
    }
    a.dirty = false;
    let image = egui::ColorImage::new([a.side, a.side], a.pixels.clone());
    match &mut a.texture {
        Some(t) => t.set(image, egui::TextureOptions::LINEAR),
        None => a.texture = Some(ctx.load_texture("skin", image, egui::TextureOptions::LINEAR)),
    }
}
/// `look` covering `rect` (its shadows reach outside), faded to `alpha`.
fn chrome(ctx: &egui::Context, look: Look, seg: Seg, rect: Rect, alpha: f32) -> Shape {
    if alpha <= 0.003 || !rect.is_positive() {
        return Shape::Noop;
    }
    let ppp = ctx.pixels_per_point();
    // Whole pixels, so the texture lands texel on pixel.
    let x0 = (rect.left() * ppp).round();
    let y0 = (rect.top() * ppp).round();
    let w = ((rect.width() * ppp).round()).max(1.);
    let h = ((rect.height() * ppp).round()).max(1.);
    let key = match look.stretch() {
        Stretch::None => (look, seg, w as u16, h as u16),
        Stretch::Wide => (look, seg, 0, h as u16),
        Stretch::Both => (look, seg, 0, 0),
    };
    let atlas = atlas(ctx);
    let mut a = atlas.lock();
    let Some(e) = a.entry(key, ppp, LINEAR.get()) else {
        return Shape::Noop;
    };
    if a.texture.is_none() {
        let blank = egui::ColorImage::filled([a.side, a.side], Color32::TRANSPARENT);
        a.texture = Some(ctx.load_texture("skin", blank, egui::TextureOptions::LINEAR));
    }
    let texture = a.texture.as_ref().unwrap().id();
    let side = a.side as f32;
    drop(a);
    // Screen and texture stops along one axis: both ends as drawn, and between them the
    // middle texel's centre stretched over what is left.
    let stops = |from: f32, length: f32, at: usize, size: usize, cap: usize| {
        let (pad, at, size) = (e.pad as f32, at as f32, size as f32);
        let (s0, s1) = (from - pad, from + length + pad);
        if cap == 0 {
            return vec![(s0, s1, at, at + size)];
        }
        let cap = (cap as f32).min(((s1 - s0) / 2.).floor());
        let middle = at + (size - 1.) / 2. + 0.5;
        vec![
            (s0, s0 + cap, at, at + cap),
            (s0 + cap, s1 - cap, middle, middle),
            (s1 - cap, s1, at + size - cap, at + size),
        ]
    };
    let mut mesh = egui::Mesh::with_texture(texture);
    let tint = Color32::WHITE.gamma_multiply(alpha.min(1.));
    for (sy0, sy1, v0, v1) in stops(y0, h, e.at[1], e.size[1], e.cap[1]) {
        for &(sx0, sx1, u0, u1) in &stops(x0, w, e.at[0], e.size[0], e.cap[0]) {
            if sx1 > sx0 && sy1 > sy0 {
                mesh.add_rect_with_uv(
                    Rect::from_min_max(pos2(sx0, sy0) / ppp, pos2(sx1, sy1) / ppp),
                    Rect::from_min_max(pos2(u0, v0) / side, pos2(u1, v1) / side),
                    tint,
                );
            }
        }
    }
    Shape::mesh(mesh)
}
fn paint(ui: &Ui, look: Look, seg: Seg, rect: Rect, alpha: f32) {
    let shape = chrome(ui.ctx(), look, seg, rect, alpha);
    ui.painter().add(shape);
}

// ---------------------------------------------------------------------------------------------
// Shared pieces of the widgets.

/// The board's eased curve, cubic-bezier(.3, .7, .3, 1): the switch knob and the chevron.
fn ease(t: f32) -> f32 {
    let bezier = |s: f32, a: f32, b: f32| {
        3. * (1. - s) * (1. - s) * s * a + 3. * (1. - s) * s * s * b + s * s * s
    };
    let (mut low, mut high) = (0., 1.);
    for _ in 0..14 {
        let mid = (low + high) / 2.;
        if bezier(mid, 0.3, 0.3) < t {
            low = mid;
        } else {
            high = mid;
        }
    }
    bezier((low + high) / 2., 0.7, 1.)
}
/// 0..1 following `on` over `seconds`; it asks for repaints only while it moves.
fn fade(ui: &Ui, id: egui::Id, what: &str, on: bool, seconds: f32) -> f32 {
    ui.ctx().animate_bool_with_time(id.with(what), on, seconds)
}
/// How far a control is hovered and pressed, 0..1 each.
fn hover_press(ui: &Ui, r: &Response) -> (f32, f32) {
    if let Some(f) = FORCED.get() {
        return (f.hover as u8 as f32, f.press as u8 as f32);
    }
    let enabled = ui.is_enabled();
    (
        fade(ui, r.id, "hover", enabled && r.hovered(), 0.1),
        fade(
            ui,
            r.id,
            "press",
            enabled && r.is_pointer_button_down_on(),
            0.06,
        ),
    )
}
fn blend(a: Color32, b: Color32, t: f32) -> Color32 {
    mix(a, b, t.clamp(0., 1.))
}
/// One line of text, cut with an ellipsis at `width`.
fn line(ui: &Ui, text: &str, font: egui::FontId, color: Color32, width: f32) -> Arc<egui::Galley> {
    let mut job = egui::text::LayoutJob::simple_singleline(text.to_owned(), font, color);
    job.wrap = egui::text::TextWrapping::truncate_at_width(width.max(0.));
    ui.painter().layout_job(job)
}
/// An icon stroke on the board's 24-unit grid, drawn 16 points across around `centre` and
/// turned by `angle`.
fn stroke_icon(ui: &Ui, centre: Pos2, points: &[[f32; 2]], angle: f32, width: f32, color: Color32) {
    let turn = egui::emath::Rot2::from_angle(angle);
    let points = points
        .iter()
        .map(|p| centre + turn * (vec2(p[0] - 12., p[1] - 12.) * (16. / 24.)))
        .collect();
    ui.painter()
        .add(Shape::line(points, egui::Stroke::new(width, color)));
}
const CHEVRON_RIGHT: [[f32; 2]; 3] = [[9., 6.], [15., 12.], [9., 18.]];
const CHEVRON_DOWN: [[f32; 2]; 3] = [[6., 9.], [12., 15.], [18., 9.]];

// ---------------------------------------------------------------------------------------------
// Buttons.

#[derive(Clone, Copy, PartialEq)]
enum Chrome {
    Raised,
    Flat,
    Primary,
}
struct Push<'a> {
    text: &'a str,
    chrome: Chrome,
    /// A toggle that is on: pressed in, with accent text.
    on: bool,
    /// Held down by something else (a combo whose menu is open).
    held: bool,
    seg: Seg,
    /// None: as wide as the text.
    width: Option<f32>,
    /// The combo's arrow at the right.
    chevron: bool,
    /// The text's side padding.
    pad: f32,
    height: f32,
    /// Fixed-pitch type, for a row whose columns line up.
    mono: bool,
    /// The text at the left even without the combo's arrow.
    left: bool,
}
impl<'a> Push<'a> {
    fn new(text: &'a str) -> Self {
        Self {
            text,
            chrome: Chrome::Raised,
            on: false,
            held: false,
            seg: Seg::Solo,
            width: None,
            chevron: false,
            pad: 9.,
            height: CONTROL,
            mono: false,
            left: false,
        }
    }
    fn font(&self) -> egui::FontId {
        if self.mono {
            mono(12.)
        } else if self.chrome == Chrome::Primary {
            semibold(12.5)
        } else {
            sans(12.5)
        }
    }
    /// The arrow and its gap.
    fn arrow(&self) -> f32 {
        if self.chevron { 6. + 2. + 16. } else { 0. }
    }
    fn natural_width(&self, ui: &Ui) -> f32 {
        let text = line(ui, self.text, self.font(), TX, f32::INFINITY);
        (text.size().x + 2. * self.pad + self.arrow()).ceil()
    }
    fn show(self, ui: &mut Ui) -> Response {
        let width = self.width.unwrap_or_else(|| self.natural_width(ui));
        let (rect, r) = ui.allocate_exact_size(vec2(width, self.height), Sense::click());
        r.widget_info(|| {
            egui::WidgetInfo::selected(egui::Role::Button, ui.is_enabled(), self.on, self.text)
        });
        if !ui.is_rect_visible(rect.expand(4.)) {
            return r;
        }
        let press = paint_push(ui, rect, &r, self.chrome, self.on, self.held, self.seg);
        let color = match (self.chrome, self.on) {
            (Chrome::Primary, _) => INK,
            (_, true) => ACC,
            _ => TX,
        };
        // Pressed content sits half a point lower.
        let down = 0.5 * press;
        let room = rect.width() - 2. * self.pad - self.arrow();
        let text = line(ui, self.text, self.font(), color, room);
        // Centred, or at the left when the button is wider than its text (a combo).
        let x = if self.chevron || self.left {
            rect.left() + self.pad
        } else {
            rect.center().x - text.size().x / 2.
        };
        ui.painter().galley(
            pos2(x, rect.center().y - text.size().y / 2. + down),
            text,
            color,
        );
        if self.chevron {
            let centre = pos2(rect.right() - self.pad - 8., rect.center().y + down);
            stroke_icon(ui, centre, &CHEVRON_DOWN, 0., 1.2, FAINT);
        }
        r
    }
}
/// The chrome of a push button on `rect`: its rest look, the hover and pressed looks faded in
/// by the pointer, and the focus ring. Returns how far it is pressed, 0..1.
fn paint_push(
    ui: &Ui,
    rect: Rect,
    r: &Response,
    chrome: Chrome,
    on: bool,
    held: bool,
    seg: Seg,
) -> f32 {
    let (hover, mut press) = hover_press(ui, r);
    if held {
        press = 1.;
    }
    let (rest, hovered, pressed) = match (chrome, on) {
        (Chrome::Primary, _) => (
            Some(Look::Primary),
            Look::PrimaryHover,
            Look::PrimaryPressed,
        ),
        (_, true) => (Some(Look::On), Look::OnHover, Look::OnPressed),
        (Chrome::Raised, false) => (Some(Look::Raised), Look::RaisedHover, Look::Pressed),
        (Chrome::Flat, false) => (None, Look::FlatHover, Look::Pressed),
    };
    if let Some(rest) = rest {
        paint(ui, rest, seg, rect, 1.);
    }
    paint(ui, hovered, seg, rect, hover);
    paint(ui, pressed, seg, rect, press);
    if focused(r) {
        let ring = if chrome == Chrome::Primary {
            Look::RingPrimary
        } else {
            Look::Ring(4)
        };
        paint(ui, ring, Seg::Solo, rect, 1.);
    }
    press
}
/// The button chrome for a control the caller draws itself (an icon button): paints `rect`
/// with the rest, hover, pressed and focus looks of `response`, flat until the pointer is on it
/// when `flat`, pressed in while `on`. Returns how far down the content sits (half a point
/// while pressed): add it to the content's y. The rect's height is the control's.
pub(crate) fn button_chrome(ui: &Ui, rect: Rect, response: &Response, flat: bool, on: bool) -> f32 {
    let chrome = if flat { Chrome::Flat } else { Chrome::Raised };
    0.5 * paint_push(ui, rect, response, chrome, on, false, Seg::Solo)
}
/// What a caller hands over as egui widget text, as the plain text the skin draws.
fn plain(text: impl Into<egui::WidgetText>) -> String {
    text.into().text().to_owned()
}
/// A raised button; in a menu, a menu row.
pub(crate) fn button(ui: &mut Ui, text: impl Into<egui::WidgetText>) -> Response {
    Button::new(text).show(ui)
}
/// `button`, lower and tighter, for a button inside a line of text or a list row.
pub(crate) fn small_button(ui: &mut Ui, text: impl Into<egui::WidgetText>) -> Response {
    Button::new(text).small().show(ui)
}
/// One of a set of choices outside a combo: flat, pressed in while chosen. In a menu, a row.
pub(crate) fn selectable(ui: &mut Ui, on: bool, text: impl Into<egui::WidgetText>) -> Response {
    Button::new(text).frame(false).selected(on).show(ui)
}
/// `selectable` for a list row of fixed-pitch text (columns that line up): the full width of
/// the list, the text at the left.
pub(crate) fn selectable_mono(ui: &mut Ui, on: bool, text: &str) -> Response {
    Push {
        chrome: Chrome::Flat,
        on,
        mono: true,
        left: true,
        height: 22.,
        width: Some(ui.available_width()),
        ..Push::new(text)
    }
    .show(ui)
}
/// `selectable` that sets `current` to `value`.
pub(crate) fn selectable_value<T: PartialEq>(
    ui: &mut Ui,
    current: &mut T,
    value: T,
    text: impl Into<egui::WidgetText>,
) -> Response {
    let mut r = selectable(ui, *current == value, text);
    if r.clicked() && *current != value {
        *current = value;
        r.mark_changed();
    }
    r
}
/// A button built the way `egui::Button` is, for the call sites that set it up in steps.
pub(crate) struct Button {
    text: String,
    shortcut: String,
    flat: bool,
    small: bool,
    selected: bool,
}
impl Button {
    pub(crate) fn new(text: impl Into<egui::WidgetText>) -> Self {
        Self {
            text: plain(text),
            shortcut: String::new(),
            flat: false,
            small: false,
            selected: false,
        }
    }
    /// A menu row's key, at its right.
    pub(crate) fn shortcut_text(mut self, text: impl Into<egui::WidgetText>) -> Self {
        self.shortcut = plain(text);
        self
    }
    pub(crate) fn frame(mut self, frame: bool) -> Self {
        self.flat = !frame;
        self
    }
    pub(crate) fn small(mut self) -> Self {
        self.small = true;
        self
    }
    pub(crate) fn selected(mut self, selected: bool) -> Self {
        self.selected = selected;
        self
    }
    fn show(self, ui: &mut Ui) -> Response {
        if egui::containers::menu::is_in_menu(ui) {
            return item_row(ui, self.selected, &self.text, &self.shortcut, false, false);
        }
        Push {
            chrome: if self.flat {
                Chrome::Flat
            } else {
                Chrome::Raised
            },
            on: self.selected,
            height: if self.small { 20. } else { CONTROL },
            pad: if self.small { 6. } else { 9. },
            ..Push::new(&self.text)
        }
        .show(ui)
    }
}
impl egui::Widget for Button {
    fn ui(self, ui: &mut Ui) -> Response {
        self.show(ui)
    }
}
/// The one button a card is about: the accent.
#[allow(dead_code)] // On the sheet; the app's cards have no accent button yet.
pub(crate) fn primary(ui: &mut Ui, text: &str) -> Response {
    Push {
        chrome: Chrome::Primary,
        ..Push::new(text)
    }
    .show(ui)
}
/// A button with no chrome until the pointer is on it; pressed in while `on`.
pub(crate) fn flat(ui: &mut Ui, on: bool, text: &str) -> Response {
    Push {
        chrome: Chrome::Flat,
        on,
        ..Push::new(text)
    }
    .show(ui)
}
/// A raised button that stays pressed in, with accent text, while `on`.
pub(crate) fn toggle(ui: &mut Ui, on: bool, text: &str) -> Response {
    Push {
        on,
        ..Push::new(text)
    }
    .show(ui)
}
/// Buttons joined into one control; the one equal to `value` is pressed in. True when a click
/// changed `value`. The padding gives way in a column too narrow for the full width.
pub(crate) fn segment<T: PartialEq + Copy>(
    ui: &mut Ui,
    value: &mut T,
    options: &[(T, &str)],
) -> bool {
    let tipped: Vec<(T, &str, &str)> = options.iter().map(|(v, t)| (*v, *t, "")).collect();
    segment_tipped(ui, value, &tipped)
}
/// `segment` with a hover text per button (empty for none).
pub(crate) fn segment_tipped<T: PartialEq + Copy>(
    ui: &mut Ui,
    value: &mut T,
    options: &[(T, &str, &str)],
) -> bool {
    let natural: Vec<f32> = options
        .iter()
        .map(|(_, text, _)| Push::new(text).natural_width(ui))
        .collect();
    // Neighbours share a border.
    let joins = options.len().saturating_sub(1) as f32;
    let total = natural.iter().sum::<f32>() - joins;
    let squeeze = ((total - ui.available_width()).max(0.) / (2. * options.len() as f32))
        .ceil()
        .min(5.);
    let width = total - squeeze * 2. * options.len() as f32;
    let mut changed = false;
    ui.scope(|ui| {
        ui.spacing_mut().item_spacing.x = -1.;
        let shadow = ui.painter().add(Shape::Noop);
        for (i, (option, text, tip)) in options.iter().enumerate() {
            let seg = match i {
                _ if options.len() == 1 => Seg::Solo,
                0 => Seg::First,
                _ if i + 1 == options.len() => Seg::Last,
                _ => Seg::Middle,
            };
            let r = Push {
                on: *value == *option,
                seg,
                pad: 9. - squeeze,
                width: Some(natural[i] - 2. * squeeze),
                ..Push::new(text)
            }
            .show(ui);
            if !tip.is_empty() {
                r.clone().on_hover_text(*tip);
            }
            if r.clicked() && *value != *option {
                *value = *option;
                changed = true;
            }
            if i == 0 {
                let group = Rect::from_min_size(r.rect.min, vec2(width, CONTROL));
                let under = chrome(ui.ctx(), Look::SegmentShadow, Seg::Solo, group, 1.);
                ui.painter().set(shadow, under);
            }
        }
    });
    changed
}

// ---------------------------------------------------------------------------------------------
// Combo and menu.

/// A combo box as wide as the room left: the current choice and an arrow; a click opens a
/// menu that `add` fills with `item`s.
pub(crate) fn combo(ui: &mut Ui, text: &str, add: impl FnOnce(&mut Ui)) -> Response {
    let width = ui.available_width().max(48.);
    let id = ui.next_auto_id();
    let open = egui::Popup::is_id_open(ui.ctx(), id.with("popup"));
    let r = Push {
        width: Some(width),
        chevron: true,
        held: open,
        ..Push::new(text)
    }
    .show(ui);
    egui::Popup::menu(&r)
        .id(id.with("popup"))
        .gap(2.)
        .frame(egui::Frame::NONE)
        .width(r.rect.width())
        .show(|ui| menu_at_least(ui, r.rect.width(), add));
    r
}
/// A combo built the way `egui::ComboBox` is. In a row of the properties panel it fills the
/// controls column unless given a width; elsewhere it is as wide as its text.
pub(crate) struct ComboBox {
    id: Option<egui::Id>,
    label: String,
    text: String,
    width: Option<f32>,
    height: f32,
}
impl ComboBox {
    pub(crate) fn from_id_salt(salt: impl std::hash::Hash + std::fmt::Debug) -> Self {
        Self {
            id: Some(egui::Id::unique(salt)),
            label: String::new(),
            text: String::new(),
            width: None,
            height: 420.,
        }
    }
    /// The label follows the combo, as egui's does.
    pub(crate) fn from_label(label: impl Into<egui::WidgetText>) -> Self {
        let label = plain(label);
        Self {
            label: label.clone(),
            ..Self::from_id_salt(label)
        }
    }
    pub(crate) fn selected_text(mut self, text: impl Into<egui::WidgetText>) -> Self {
        self.text = plain(text);
        self
    }
    pub(crate) fn width(mut self, width: f32) -> Self {
        self.width = Some(width);
        self
    }
    /// The menu's greatest height.
    pub(crate) fn height(mut self, height: f32) -> Self {
        self.height = height;
        self
    }
    pub(crate) fn show_ui<R>(
        mut self,
        ui: &mut Ui,
        add: impl FnOnce(&mut Ui) -> R,
    ) -> egui::InnerResponse<Option<R>> {
        if !self.label.is_empty() {
            // The label follows on the same line.
            let label = std::mem::take(&mut self.label);
            return ui
                .horizontal(|ui| {
                    let r = self.show_ui(ui, add);
                    ui.label(label);
                    r
                })
                .inner;
        }
        let push = Push {
            chevron: true,
            ..Push::new(&self.text)
        };
        let room = ui.available_width();
        let width = match self.width {
            Some(width) => width.min(room.max(48.)),
            None if fills() && super::in_row(ui.ctx()) => room.max(48.),
            None => push.natural_width(ui).max(72.).min(room.max(48.)),
        };
        let id = ui.make_persistent_id(self.id).with("popup");
        let r = Push {
            width: Some(width),
            held: egui::Popup::is_id_open(ui.ctx(), id),
            ..push
        }
        .show(ui);
        let popup = egui::Popup::menu(&r)
            .id(id)
            .gap(2.)
            .frame(egui::Frame::NONE)
            .width(width);
        let height = self.height;
        let inner = popup
            .show(|ui| menu_sized(ui, width, height, add))
            .map(|shown| shown.inner);
        egui::InnerResponse::new(inner, r)
    }
}
/// A menu's body: the popup chrome behind `add`'s items.
pub(crate) fn menu<R>(ui: &mut Ui, add: impl FnOnce(&mut Ui) -> R) -> R {
    menu_at_least(ui, 0., add)
}
/// `menu`, no narrower than `width` (the combo it hangs from).
fn menu_at_least<R>(ui: &mut Ui, width: f32, add: impl FnOnce(&mut Ui) -> R) -> R {
    menu_sized(ui, width, 420., add)
}
fn menu_sized<R>(ui: &mut Ui, width: f32, height: f32, add: impl FnOnce(&mut Ui) -> R) -> R {
    let behind = ui.painter().add(Shape::Noop);
    let body = egui::Frame::NONE.inner_margin(4.).show(ui, |ui| {
        ui.spacing_mut().item_spacing = Vec2::ZERO;
        ui.set_min_width((width - 8.).max(0.));
        // The rows fill the width the menu was measured at. Without this they fill whatever
        // the popup offers, and a list long enough to scroll adds its scroll bar to that
        // every frame, so the menu grew to the window's width.
        let key = ui.scope_id().with("menu rows width");
        let measured = ui.data(|d| d.get_temp::<f32>(key));
        let sizing = ui.is_sizing_pass();
        let shown = egui::ScrollArea::vertical()
            .max_height(height)
            .show(ui, |ui| {
                if let Some(measured) = measured.filter(|_| !sizing) {
                    ui.set_max_width(measured);
                }
                add(ui)
            });
        if sizing || measured.is_none() {
            ui.data_mut(|d| d.insert_temp(key, shown.content_size.x));
        }
        shown.inner
    });
    let shape = chrome(ui.ctx(), Look::Popup, Seg::Solo, body.response.rect, 1.);
    ui.painter().set(behind, shape);
    body.inner
}
/// A menu's row: filled with the accent under the pointer; the current choice is in the
/// accent colour.
pub(crate) fn item(ui: &mut Ui, selected: bool, text: impl Into<egui::WidgetText>) -> Response {
    item_row(ui, selected, &plain(text), "", false, false)
}
/// `item` with a key at its right (a menu's shortcut), or an arrow for a submenu (`arrow`),
/// filled like a hovered row while its submenu is `open`.
fn item_row(
    ui: &mut Ui,
    selected: bool,
    text: &str,
    key: &str,
    arrow: bool,
    open: bool,
) -> Response {
    let label = line(ui, text, sans(12.5), TX, f32::INFINITY);
    let key_width = if arrow {
        8. + 16.
    } else if key.is_empty() {
        0.
    } else {
        line(ui, key, mono(11.), FAINT, f32::INFINITY).size().x + 24.
    };
    let natural = label.size().x + 18. + key_width;
    let width = if ui.is_sizing_pass() {
        natural
    } else {
        ui.available_width().max(natural)
    };
    let (rect, r) = ui.allocate_exact_size(vec2(width, CONTROL), Sense::click());
    r.widget_info(|| {
        egui::WidgetInfo::selected(egui::Role::Button, ui.is_enabled(), selected, text)
    });
    if ui.is_rect_visible(rect) {
        let (hover, _) = hover_press(ui, &r);
        let hover = if open { 1. } else { hover };
        if hover > 0. {
            ui.painter()
                .rect_filled(rect, 3., ACC.gamma_multiply(hover));
        }
        let color = blend(if selected { ACC } else { TX }, INK, hover);
        let label = line(ui, text, sans(12.5), color, f32::INFINITY);
        ui.painter().galley(
            pos2(rect.left() + 9., rect.center().y - label.size().y / 2.),
            label,
            color,
        );
        if arrow {
            let centre = pos2(rect.right() - 9. - 8., rect.center().y);
            let color = blend(FAINT, INK, hover);
            stroke_icon(ui, centre, &CHEVRON_RIGHT, 0., 1.2, color);
        } else if !key.is_empty() {
            let color = blend(FAINT, INK, hover);
            let key = line(ui, key, mono(11.), color, f32::INFINITY);
            ui.painter().galley(
                pos2(
                    rect.right() - 9. - key.size().x,
                    rect.center().y - key.size().y / 2.,
                ),
                key,
                color,
            );
        }
    }
    r
}
/// An `item` that sets `current` to `value`.
pub(crate) fn item_value<T: PartialEq>(
    ui: &mut Ui,
    current: &mut T,
    value: T,
    text: impl Into<egui::WidgetText>,
) -> Response {
    let mut r = item(ui, *current == value, text);
    if r.clicked() && *current != value {
        *current = value;
        r.mark_changed();
    }
    r
}
/// The line between a menu's groups: black over light.
pub(crate) fn separator(ui: &mut Ui) {
    // A menu is sized by its rows: while it is being measured the line has no width of its own.
    let width = if ui.is_sizing_pass() {
        0.
    } else {
        ui.available_width()
    };
    let (rect, _) = ui.allocate_exact_size(vec2(width, 8.), Sense::hover());
    let y = rect.center().y;
    let row = |from: f32, color| {
        ui.painter().rect_filled(
            Rect::from_min_max(pos2(rect.left(), from), pos2(rect.right(), from + 1.)),
            0.,
            color,
        );
    };
    row(y - 1., EDGE);
    row(y, LIGHT_LINE);
}

// ---------------------------------------------------------------------------------------------
// Menus, context menus, cards and windows.

/// The style a menu's popup runs with. egui builds the popup's frame from it (`Frame::popup`)
/// and then the menu's own ui, so the stock frame is made invisible here and `menu_content`
/// paints the board's menu behind the rows: 4 points of padding, rows with no gaps.
fn menu_look(s: &mut egui::Style) {
    egui::containers::menu::menu_style(s);
    s.spacing.item_spacing = Vec2::ZERO;
    s.spacing.menu_margin = egui::Margin::same(4);
    s.visuals.window_fill = Color32::TRANSPARENT;
    s.visuals.window_stroke = egui::Stroke::NONE;
    s.visuals.popup_shadow = egui::Shadow::NONE;
}
/// What a menu's popup shows: the rows from `add`, on the board's menu chrome (`Look::Popup`).
/// The popup's own frame is invisible under `menu_look` and holds the 4 points the chrome
/// reaches over.
fn menu_content<R>(ui: &mut Ui, add: impl FnOnce(&mut Ui) -> R) -> R {
    let behind = ui.painter().add(Shape::Noop);
    let inner = add(ui);
    let body = ui.min_rect().expand(4.);
    ui.painter()
        .set(behind, chrome(ui.ctx(), Look::Popup, Seg::Solo, body, 1.));
    inner
}
/// Is the menu hanging from `button` open?
pub(crate) fn menu_open(button: &Response) -> bool {
    egui::Popup::is_id_open(&button.ctx, egui::Popup::default_response_id(button))
}
/// Opens a menu of `add`'s rows from `button` when it is clicked. `close` is when a click
/// closes the menu (None: as the menu this is in, plain click by default); a menu that holds
/// checkboxes closes only on a click outside.
pub(crate) fn menu_on<R>(
    ui: &Ui,
    button: &Response,
    close: Option<egui::PopupCloseBehavior>,
    add: impl FnOnce(&mut Ui) -> R,
) -> Option<egui::InnerResponse<R>> {
    let close =
        close.unwrap_or_else(|| egui::containers::menu::MenuConfig::find(ui).close_behavior);
    egui::Popup::menu(button)
        .close_behavior(close)
        .style(menu_look)
        .show(|ui| menu_content(ui, add))
}
/// A context menu on a right click of `response`.
pub(crate) fn context_menu<R>(
    response: &Response,
    add: impl FnOnce(&mut Ui) -> R,
) -> Option<egui::InnerResponse<R>> {
    egui::Popup::context_menu(response)
        .style(menu_look)
        .show(|ui| menu_content(ui, add))
}
/// A button that opens a menu of `add`'s rows. Outside a menu a flat button, pressed in while
/// its menu is open; inside one a row with an arrow that opens a submenu at its right.
pub(crate) fn menu_button<R>(
    ui: &mut Ui,
    text: impl Into<egui::WidgetText>,
    add: impl FnOnce(&mut Ui) -> R,
) -> egui::InnerResponse<Option<R>> {
    menu_button_with(ui, &plain(text), false, None, add)
}
/// `menu_button` with the raised look, for a button among raised ones.
pub(crate) fn menu_button_raised<R>(
    ui: &mut Ui,
    text: impl Into<egui::WidgetText>,
    add: impl FnOnce(&mut Ui) -> R,
) -> egui::InnerResponse<Option<R>> {
    menu_button_with(ui, &plain(text), true, None, add)
}
/// `menu_button` (raised) whose menu stays open while its widgets are used: only a click
/// outside closes it, and plain items call `ui.close()`.
pub(crate) fn menu_button_sticky<R>(
    ui: &mut Ui,
    text: impl Into<egui::WidgetText>,
    add: impl FnOnce(&mut Ui) -> R,
) -> egui::InnerResponse<Option<R>> {
    let close = Some(egui::PopupCloseBehavior::CloseOnClickOutside);
    menu_button_with(ui, &plain(text), true, close, add)
}
fn menu_button_with<R>(
    ui: &mut Ui,
    text: &str,
    raised: bool,
    close: Option<egui::PopupCloseBehavior>,
    add: impl FnOnce(&mut Ui) -> R,
) -> egui::InnerResponse<Option<R>> {
    use egui::containers::menu::{MenuState, SubMenu};
    if egui::containers::menu::is_in_menu(ui) {
        // A submenu keeps the menu it hangs from (and so its look and close behaviour).
        let id = ui.next_auto_id();
        let open = MenuState::from_ui(ui, |state, _| {
            state.open_item == Some(SubMenu::id_from_widget_id(id))
        });
        let r = item_row(ui, false, text, "", true, open);
        let inner = SubMenu::new().show(ui, &r, |ui| menu_content(ui, add));
        return egui::InnerResponse::new(inner.map(|i| i.inner), r);
    }
    let id = ui.next_auto_id();
    let open = egui::Popup::is_id_open(ui.ctx(), id.with("popup"));
    let r = Push {
        chrome: if raised { Chrome::Raised } else { Chrome::Flat },
        held: open,
        ..Push::new(text)
    }
    .show(ui);
    let inner = menu_on(ui, &r, close, add);
    egui::InnerResponse::new(inner.map(|i| i.inner), r)
}
/// The colour button: the colour in a one-point edge, ringed while its picker is open.
fn swatch(ui: &mut Ui, color: Color32, open: bool) -> Response {
    let (rect, r) = ui.allocate_exact_size(ui.spacing().interact_size, Sense::click());
    r.widget_info(|| egui::WidgetInfo::labeled(egui::Role::ColorWell, ui.is_enabled(), "Color"));
    if ui.is_rect_visible(rect) {
        let (hover, _) = hover_press(ui, &r);
        let edge = blend(EDGE, LIGHT_LINE, hover);
        ui.painter().rect_filled(rect, 4., color);
        ui.painter().rect_stroke(
            rect,
            4.,
            egui::Stroke::new(1_f32, edge),
            egui::StrokeKind::Inside,
        );
        if open || focused(&r) {
            paint(ui, Look::Ring(4), Seg::Solo, rect, 1.);
        }
    }
    r
}
/// A colour swatch that opens a picker in the board's menu, for a colour kept as plain RGB.
/// The hue is kept while the colour is grey.
pub(crate) fn color_rgb(ui: &mut Ui, rgb: &mut [f32; 3]) -> Response {
    let id = ui.auto_id_with("colour");
    let now = egui::ecolor::Rgba::from_rgb(rgb[0], rgb[1], rgb[2]);
    let open = egui::Popup::is_id_open(ui.ctx(), id.with("popup"));
    let mut hsva = ui
        .data(|d| d.get_temp::<egui::ecolor::Hsva>(id))
        .filter(|h| egui::ecolor::Rgba::from(*h) == now)
        .unwrap_or_else(|| egui::ecolor::Hsva::from(now));
    let mut button = swatch(ui, Color32::from(hsva), open);
    if ui.style().explanation_tooltips {
        button = button.on_hover_text("Click to edit color");
    }
    egui::Popup::menu(&button)
        .id(id.with("popup"))
        .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
        .style(menu_look)
        .show(|ui| {
            menu_content(ui, |ui| {
                ui.spacing_mut().item_spacing = vec2(8., 6.);
                ui.add_space(4.);
                if picker(ui, &mut hsva) {
                    button.mark_changed();
                }
                ui.add_space(4.);
            })
        });
    // Only an edit writes back: the round trip through HSV is not bit-exact, and a colour
    // that changes by a rounding error restarts the render.
    if button.changed() {
        let out = egui::ecolor::Rgba::from(hsva);
        *rgb = [out.r(), out.g(), out.b()];
    }
    ui.data_mut(|d| d.insert_temp(id, hsva));
    button
}
/// The picker's width: the saturation/value square, the hue strip and the R, G, B fields.
const PICKER: f32 = 216.;
/// The picker in the menu: a saturation/value square, a hue strip and the three channels as
/// numbers. True when the colour changed.
fn picker(ui: &mut Ui, hsva: &mut egui::ecolor::Hsva) -> bool {
    let before = *hsva;
    // Saturation to the right, value up. The square is a grid of quads: the colour is
    // bilinear in both, so a coarse grid draws it exactly enough.
    let (square, r) = ui.allocate_exact_size(vec2(PICKER, 132.), Sense::click_and_drag());
    if let Some(p) = r
        .interact_pointer_pos()
        .filter(|_| r.is_pointer_button_down_on())
    {
        hsva.s = ((p.x - square.left()) / square.width()).clamp(0., 1.);
        hsva.v = 1. - ((p.y - square.top()) / square.height()).clamp(0., 1.);
    }
    let (strip, r) = ui.allocate_exact_size(vec2(PICKER, 14.), Sense::click_and_drag());
    if let Some(p) = r
        .interact_pointer_pos()
        .filter(|_| r.is_pointer_button_down_on())
    {
        hsva.h = ((p.x - strip.left()) / strip.width()).clamp(0., 1.);
    }
    if ui.is_rect_visible(square) {
        const N: usize = 16;
        let mut mesh = egui::Mesh::default();
        for j in 0..=N {
            for i in 0..=N {
                let (s, v) = (i as f32 / N as f32, 1. - j as f32 / N as f32);
                mesh.colored_vertex(
                    pos2(
                        square.left() + s * square.width(),
                        square.top() + (1. - v) * square.height(),
                    ),
                    egui::ecolor::Hsva::new(hsva.h, s, v, 1.).into(),
                );
            }
        }
        for j in 0..N as u32 {
            for i in 0..N as u32 {
                let a = j * (N as u32 + 1) + i;
                let b = a + N as u32 + 1;
                mesh.add_triangle(a, a + 1, b);
                mesh.add_triangle(a + 1, b + 1, b);
            }
        }
        let painter = ui.painter();
        painter.add(Shape::mesh(mesh));
        painter.rect_stroke(
            square,
            0.,
            egui::Stroke::new(1_f32, EDGE),
            egui::StrokeKind::Outside,
        );
        let at = pos2(
            square.left() + hsva.s * square.width(),
            square.top() + (1. - hsva.v) * square.height(),
        );
        painter.circle_stroke(at, 5.5, egui::Stroke::new(2_f32, Color32::BLACK));
        painter.circle_stroke(at, 5.5, egui::Stroke::new(1_f32, Color32::WHITE));
    }
    if ui.is_rect_visible(strip) {
        let mut mesh = egui::Mesh::default();
        const STOPS: usize = 12;
        for i in 0..=STOPS {
            let t = i as f32 / STOPS as f32;
            let x = strip.left() + t * strip.width();
            let c: Color32 = egui::ecolor::Hsva::new(t, 1., 1., 1.).into();
            mesh.colored_vertex(pos2(x, strip.top()), c);
            mesh.colored_vertex(pos2(x, strip.bottom()), c);
        }
        for i in 0..STOPS as u32 {
            let a = i * 2;
            mesh.add_triangle(a, a + 1, a + 2);
            mesh.add_triangle(a + 1, a + 3, a + 2);
        }
        let painter = ui.painter();
        painter.add(Shape::mesh(mesh));
        painter.rect_stroke(
            strip,
            0.,
            egui::Stroke::new(1_f32, EDGE),
            egui::StrokeKind::Outside,
        );
        let x = strip.left() + hsva.h * strip.width();
        let knob = Rect::from_center_size(pos2(x, strip.center().y), vec2(6., strip.height() + 4.));
        painter.rect_stroke(
            knob,
            2.,
            egui::Stroke::new(2_f32, Color32::BLACK),
            egui::StrokeKind::Middle,
        );
        painter.rect_stroke(
            knob,
            2.,
            egui::Stroke::new(1_f32, Color32::WHITE),
            egui::StrokeKind::Middle,
        );
    }
    let shown = Color32::from(*hsva);
    let mut bytes = [shown.r(), shown.g(), shown.b()];
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 4.;
        let width = (PICKER - 2. * 4. - 3. * 14.) / 3.;
        for (name, byte) in ["R", "G", "B"].into_iter().zip(&mut bytes) {
            let (rect, _) = ui.allocate_exact_size(vec2(14., CONTROL), Sense::hover());
            let label = line(ui, name, sans(11.), FAINT, f32::INFINITY);
            ui.painter().galley(
                pos2(rect.left(), rect.center().y - label.size().y / 2.),
                label,
                FAINT,
            );
            drag_value_sized(
                ui,
                width,
                egui::DragValue::new(byte).range(0..=255).speed(0.5),
            );
        }
    });
    if bytes != [shown.r(), shown.g(), shown.b()] {
        let h = hsva.h;
        *hsva = egui::ecolor::Hsva::from(Color32::from_rgb(bytes[0], bytes[1], bytes[2]));
        // A grey has no hue of its own: the strip keeps where it was.
        if hsva.s == 0. {
            hsva.h = h;
        }
    }
    *hsva != before
}
/// A card for a piece of UI that floats over the viewport (a toast, the command palette):
/// the menu's chrome with 8 points of padding.
pub(crate) fn card<R>(ui: &mut Ui, add: impl FnOnce(&mut Ui) -> R) -> egui::InnerResponse<R> {
    let behind = ui.painter().add(Shape::Noop);
    let body = egui::Frame::NONE.inner_margin(8.).show(ui, add);
    ui.painter().set(
        behind,
        chrome(ui.ctx(), Look::Popup, Seg::Solo, body.response.rect, 1.),
    );
    body
}
/// The frame of a floating window: the panel colour, a one-point edge, 6 points of corner, a
/// deep shadow. `Window` adds the highlight under the top edge.
pub(crate) fn window_frame() -> egui::Frame {
    egui::Frame::NONE
        .fill(PANEL)
        .stroke(egui::Stroke::new(1_f32, EDGE))
        .corner_radius(6)
        .inner_margin(egui::Margin::same(10))
        .shadow(egui::Shadow {
            offset: [0, 14],
            blur: 36,
            spread: 0,
            color: Color32::from_black_alpha(180),
        })
}
/// The frame of a window that fills a part of the screen (the scenes and assets browsers): the
/// panel colour and edge, no padding or shadow.
pub(crate) fn panel_frame() -> egui::Frame {
    egui::Frame::NONE
        .fill(PANEL)
        .stroke(egui::Stroke::new(1_f32, EDGE))
}
/// The face of a browser card over `rect`: the band-coloured tile, lighter under the pointer,
/// with the accent ring while `selected`. Draw it first; the thumbnail goes on top.
pub(crate) fn card_face(ui: &Ui, r: &Response, rect: Rect, selected: bool) {
    let (hover, _) = hover_press(ui, r);
    paint(ui, Look::Card, Seg::Solo, rect, 1.);
    paint(ui, Look::CardHover, Seg::Solo, rect, hover);
    if selected {
        paint(ui, Look::Ring(5), Seg::Solo, rect, 1.);
    }
}
/// A group heading in a side list or a card: 11 semibold caps, tracked, in the faint grey.
pub(crate) fn heading(ui: &mut Ui, text: &str) -> Response {
    let job = egui::text::LayoutJob::single_section(
        text.to_uppercase(),
        egui::TextFormat {
            font_id: semibold(11.),
            extra_letter_spacing: 0.66,
            color: FAINT,
            ..Default::default()
        },
    );
    ui.label(job)
}
/// A group heading between a menu's rows, its text where the rows' text starts.
pub(crate) fn menu_heading(ui: &mut Ui, text: &str) {
    let mut job = egui::text::LayoutJob::single_section(
        text.to_uppercase(),
        egui::TextFormat {
            font_id: semibold(11.),
            extra_letter_spacing: 0.66,
            color: FAINT,
            ..Default::default()
        },
    );
    job.wrap = egui::text::TextWrapping::truncate_at_width(f32::INFINITY);
    let label = ui.painter().layout_job(job);
    let (rect, _) = ui.allocate_exact_size(vec2(label.size().x + 18., 22.), Sense::hover());
    ui.painter().galley(
        pos2(rect.left() + 9., rect.center().y - label.size().y / 2.),
        label,
        FAINT,
    );
}
/// A progress bar: the accent fill in a recessed groove, `text` (a size, a percentage) in
/// Plex Mono at the middle. `width` and `height` are of the whole bar.
pub(crate) fn bar(ui: &mut Ui, fraction: f32, width: f32, height: f32, text: &str) -> Response {
    let (rect, r) = ui.allocate_exact_size(vec2(width, height), Sense::hover());
    if ui.is_rect_visible(rect) {
        paint(ui, Look::Groove, Seg::Solo, rect, 1.);
        let inner = rect.shrink(1.);
        let full = inner.width() * fraction.clamp(0., 1.);
        if full >= 2. {
            ui.painter().rect_filled(
                Rect::from_min_size(inner.min, vec2(full, inner.height())),
                2.,
                mix(ACC, Color32::BLACK, 0.22),
            );
        }
        if !text.is_empty() {
            ui.painter().text(
                rect.center(),
                egui::Align2::CENTER_CENTER,
                text,
                mono(11.),
                TX,
            );
        }
    }
    r
}
/// A floating window in the board's look. Chain the usual `egui::Window` settings and finish
/// with `show_skin` rather than `show`.
pub(crate) fn window<'a>(title: impl Into<egui::WidgetText>) -> egui::Window<'a> {
    egui::Window::new(title).frame(window_frame())
}
/// egui paints a window's collapse arrow as a filled triangle at the left of the title bar
/// and offers no way to draw another. This swaps that shape, if the window has one, for the
/// skin's chevron, open or closed as the triangle was.
fn restyle_collapse_arrow(ctx: &egui::Context, layer: egui::LayerId, window: Rect) {
    let title = Rect::from_min_size(window.min, vec2(44., 32.));
    ctx.graphics_mut(|g| {
        let list = g.entry(layer);
        let found = list.all_entries().position(|c| {
            matches!(&c.shape, Shape::Path(p)
                if p.closed && p.points.len() == 3 && p.fill.a() > 0
                    && p.points.iter().all(|q| title.contains(*q)))
        });
        let Some(i) = found else { return };
        list.mutate_shape(egui::layers::ShapeIdx(i), |c| {
            let Shape::Path(p) = &c.shape else { return };
            let centre = p.points.iter().fold(Vec2::ZERO, |sum, q| sum + q.to_vec2()) / 3.;
            // The tip is the third corner: right while closed, down while open.
            let tip = p.points[2].to_vec2() - centre;
            let turn = egui::emath::Rot2::from_angle(tip.angle());
            let line: Vec<Pos2> = CHEVRON_RIGHT
                .iter()
                .map(|q| centre.to_pos2() + turn * (vec2(q[0] - 12., q[1] - 12.) * (16. / 24.)))
                .collect();
            c.shape = Shape::line(line, egui::Stroke::new(1.2, DIM));
        });
    });
}
/// `show` for a `window`: the same, and the highlight along the window's top edge that an
/// egui frame cannot draw.
pub(crate) trait ShowSkin<'a> {
    fn show_skin<R>(
        self,
        ctx: &egui::Context,
        add: impl FnOnce(&mut Ui) -> R + 'a,
    ) -> Option<egui::InnerResponse<Option<R>>>;
}
impl<'a> ShowSkin<'a> for egui::Window<'a> {
    fn show_skin<R>(
        self,
        ctx: &egui::Context,
        add: impl FnOnce(&mut Ui) -> R + 'a,
    ) -> Option<egui::InnerResponse<Option<R>>> {
        let shown = self.show(ctx, add);
        if let Some(r) = &shown {
            let rect = r.response.rect;
            restyle_collapse_arrow(ctx, r.response.layer_id, rect);
            let radius = 6.;
            // `inset 0 1px 0` of white at 8 %: along the top inside the edge, fading out
            // round the corners.
            let white = Color32::from_white_alpha(20);
            let y = rect.top() + 1.5;
            let mut points = vec![];
            let arc = |centre: Pos2, from: f32, to: f32, points: &mut Vec<Pos2>| {
                for i in 0..=4 {
                    let a = from + (to - from) * i as f32 / 4.;
                    points.push(centre + (radius - 1.5) * vec2(a.cos(), a.sin()));
                }
            };
            use std::f32::consts::PI;
            arc(
                pos2(rect.left() + radius, rect.top() + radius),
                PI * 1.25,
                PI * 1.5,
                &mut points,
            );
            points.push(pos2(rect.right() - radius, y));
            arc(
                pos2(rect.right() - radius, rect.top() + radius),
                PI * 1.5,
                PI * 1.75,
                &mut points,
            );
            ctx.layer_painter(r.response.layer_id)
                .add(Shape::line(points, egui::Stroke::new(1_f32, white)));
        }
        shown
    }
}

// ---------------------------------------------------------------------------------------------
// Checkbox and switch.

/// A checkbox on its own (its row has the label).
pub(crate) fn checkbox(ui: &mut Ui, on: &mut bool) -> Response {
    check(ui, on, "")
}
/// A checkbox with its label; a click on either flips it.
pub(crate) fn checkbox_label(
    ui: &mut Ui,
    on: &mut bool,
    text: impl Into<egui::WidgetText>,
) -> Response {
    check(ui, on, &plain(text))
}
fn check(ui: &mut Ui, on: &mut bool, text: &str) -> Response {
    let label = (!text.is_empty()).then(|| line(ui, text, sans(12.5), TX, f32::INFINITY));
    let width = 15. + label.as_ref().map_or(0., |l| 8. + l.size().x.ceil());
    let (rect, mut r) = ui.allocate_exact_size(vec2(width, CONTROL), Sense::click());
    if r.clicked() {
        *on = !*on;
        r.mark_changed();
    }
    r.widget_info(|| egui::WidgetInfo::selected(egui::Role::CheckBox, ui.is_enabled(), *on, text));
    if !ui.is_rect_visible(rect.expand(4.)) {
        return r;
    }
    let square = Rect::from_min_size(pos2(rect.left(), rect.center().y - 7.5), Vec2::splat(15.));
    let (hover, press) = hover_press(ui, &r);
    let checked = fade(ui, r.id, "on", *on, 0.1);
    paint(ui, Look::Check, Seg::Solo, square, 1.);
    paint(ui, Look::CheckHover, Seg::Solo, square, hover);
    paint(ui, Look::CheckOn, Seg::Solo, square, checked);
    paint(ui, Look::CheckOnHover, Seg::Solo, square, checked * hover);
    if press > 0. {
        // brightness(.85) while held.
        ui.painter().rect_filled(
            square,
            3.,
            Color32::from_black_alpha((0.15 * 255. * press) as u8),
        );
    }
    if checked > 0. {
        let tick: Vec<Pos2> = [[5., 12.], [10., 17.], [19., 7.]]
            .iter()
            .map(|p| square.center() + vec2(p[0] - 12., p[1] - 12.) * (11. / 24.))
            .collect();
        let stroke = egui::Stroke::new(3.2 * 11. / 24., INK.gamma_multiply(checked));
        ui.painter().add(Shape::line(tick, stroke));
    }
    if focused(&r) {
        paint(ui, Look::Outline(3), Seg::Solo, square, 1.);
    }
    if let Some(label) = label {
        ui.painter().galley(
            pos2(square.right() + 8., rect.center().y - label.size().y / 2.),
            label,
            TX,
        );
    }
    r
}
/// A radio button with its label: one of a set, on when `on`. The caller acts on the click.
pub(crate) fn radio(ui: &mut Ui, on: bool, text: impl Into<egui::WidgetText>) -> Response {
    let text = plain(text);
    let label = (!text.is_empty()).then(|| line(ui, &text, sans(12.5), TX, f32::INFINITY));
    let width = 15. + label.as_ref().map_or(0., |l| 8. + l.size().x.ceil());
    let (rect, r) = ui.allocate_exact_size(vec2(width, CONTROL), Sense::click());
    r.widget_info(|| {
        egui::WidgetInfo::selected(egui::Role::RadioButton, ui.is_enabled(), on, &text)
    });
    if !ui.is_rect_visible(rect.expand(4.)) {
        return r;
    }
    let disc = Rect::from_min_size(pos2(rect.left(), rect.center().y - 7.5), Vec2::splat(15.));
    let (hover, press) = hover_press(ui, &r);
    let chosen = fade(ui, r.id, "on", on, 0.1);
    paint(ui, Look::Radio, Seg::Solo, disc, 1.);
    paint(ui, Look::RadioHover, Seg::Solo, disc, hover);
    paint(ui, Look::RadioOn, Seg::Solo, disc, chosen);
    paint(ui, Look::RadioOnHover, Seg::Solo, disc, chosen * hover);
    if press > 0. {
        // brightness(.85) while held, as the checkbox.
        ui.painter().circle_filled(
            disc.center(),
            7.5,
            Color32::from_black_alpha((0.15 * 255. * press) as u8),
        );
    }
    if chosen > 0. {
        ui.painter()
            .circle_filled(disc.center(), 3., INK.gamma_multiply(chosen));
    }
    if focused(&r) {
        paint(ui, Look::Outline(8), Seg::Solo, disc, 1.);
    }
    if let Some(label) = label {
        ui.painter().galley(
            pos2(disc.right() + 8., rect.center().y - label.size().y / 2.),
            label,
            TX,
        );
    }
    r
}
/// `radio` that sets `current` to `value` when clicked.
pub(crate) fn radio_value<T: PartialEq>(
    ui: &mut Ui,
    current: &mut T,
    value: T,
    text: impl Into<egui::WidgetText>,
) -> Response {
    let mut r = radio(ui, *current == value, text);
    if r.clicked() && *current != value {
        *current = value;
        r.mark_changed();
    }
    r
}
/// A switch: a setting that turns something on, as against a checkbox's option.
pub(crate) fn switch(ui: &mut Ui, on: &mut bool) -> Response {
    let (rect, mut r) = ui.allocate_exact_size(vec2(30., CONTROL), Sense::click());
    if r.clicked() {
        *on = !*on;
        r.mark_changed();
    }
    r.widget_info(|| egui::WidgetInfo::selected(egui::Role::CheckBox, ui.is_enabled(), *on, ""));
    if !ui.is_rect_visible(rect.expand(4.)) {
        return r;
    }
    let track = Rect::from_min_size(pos2(rect.left(), rect.center().y - 8.), vec2(30., 16.));
    let (hover, press) = hover_press(ui, &r);
    let lit = fade(ui, r.id, "on", *on, 0.14);
    let slide = ui
        .ctx()
        .animate_bool_with_time_and_easing(r.id.with("knob"), *on, 0.14, ease);
    paint(ui, Look::Track, Seg::Solo, track, 1.);
    paint(ui, Look::TrackOn, Seg::Solo, track, lit);
    let knob = Rect::from_min_size(track.min + vec2(2. + 14. * slide, 2.), Vec2::splat(12.));
    paint(ui, Look::Knob, Seg::Solo, knob, 1.);
    paint(ui, Look::KnobHover, Seg::Solo, knob, hover);
    paint(ui, Look::KnobPressed, Seg::Solo, knob, press);
    if focused(&r) {
        paint(ui, Look::Outline(8), Seg::Solo, track, 1.);
    }
    r
}

// ---------------------------------------------------------------------------------------------
// Fields.

/// A recessed field behind what `add` draws: one of egui's text widgets, which runs here with
/// its own chrome off and its text at the layout's side.
pub(crate) fn field(
    ui: &mut Ui,
    size: Vec2,
    layout: egui::Layout,
    add: impl FnOnce(&mut Ui) -> Response,
) -> Response {
    let behind = ui.painter().add(Shape::Noop);
    // A number with a unit can need more than its width. The room is made for it from the
    // width it came to last frame, or the field would hang out over its neighbour.
    // Only while the same width is asked for, so a field narrows again with its column.
    let key = ui.next_auto_id().with("field width");
    let asked = size.x;
    let size = vec2(
        ui.data(|d| d.get_temp::<(f32, f32)>(key))
            .filter(|(for_width, _)| *for_width == asked)
            .map_or(asked, |(_, came_to)| came_to.max(asked)),
        size.y,
    );
    let r = ui
        .allocate_ui_with_layout(size, layout, |ui| {
            bare(ui.style_mut());
            ui.spacing_mut().interact_size = size;
            ui.spacing_mut().button_padding = vec2(7., 0.);
            add(ui)
        })
        .inner;
    ui.data_mut(|d| d.insert_temp(key, (asked, r.rect.width())));
    paint_field(ui, behind, &r);
    r
}
/// The field's looks behind `r`, put in the shape slot `behind` reserved before it.
fn paint_field(ui: &Ui, behind: egui::layers::ShapeIdx, r: &Response) {
    let rect = r.rect;
    let pointer = ui.is_enabled() && ui.rect_contains_pointer(rect);
    let (hover, focus) = match FORCED.get() {
        Some(f) => (f.hover as u8 as f32, (f.focus || f.press) as u8 as f32),
        None => (
            fade(ui, r.id, "field hover", pointer, 0.1),
            fade(ui, r.id, "field focus", r.has_focus(), 0.1),
        ),
    };
    let ctx = ui.ctx();
    let shapes = vec![
        chrome(ctx, Look::Field, Seg::Solo, rect, 1.),
        chrome(ctx, Look::FieldHover, Seg::Solo, rect, hover),
        chrome(ctx, Look::FieldFocus, Seg::Solo, rect, focus),
    ];
    ui.painter().set(behind, Shape::Vec(shapes));
}
/// A one-line text field `width` points wide (the room left when that is infinite) in the
/// recessed field. `edit` keeps its id, hint, password flag and the rest; its width, margin
/// and alignment are set here.
pub(crate) fn text_edit(ui: &mut Ui, width: f32, edit: egui::TextEdit<'_>) -> Response {
    let width = if width.is_finite() {
        width
    } else {
        ui.available_width()
    };
    let layout = egui::Layout::left_to_right(egui::Align::Center);
    field(ui, vec2(width, CONTROL), layout, |ui| {
        ui.add(
            edit.desired_width(width - 14.)
                .min_size(vec2(width, CONTROL))
                .align(egui::Align2::LEFT_CENTER)
                .margin(egui::Margin::symmetric(7, 0)),
        )
    })
}
/// A multi-line text field as wide as the room left, in the recessed field; its height is
/// the edit's rows.
pub(crate) fn text_area(ui: &mut Ui, edit: egui::TextEdit<'_>) -> Response {
    let behind = ui.painter().add(Shape::Noop);
    let r = ui
        .scope(|ui| {
            bare(ui.style_mut());
            ui.add(
                edit.desired_width(f32::INFINITY)
                    .margin(egui::Margin::symmetric(7, 5)),
            )
        })
        .inner;
    paint_field(ui, behind, &r);
    r
}
/// `drag_value` at a given width, for a group that fills its column.
pub(crate) fn drag_value_sized(ui: &mut Ui, width: f32, value: egui::DragValue<'_>) -> Response {
    field(
        ui,
        vec2(width, CONTROL),
        egui::Layout::left_to_right(egui::Align::Center).with_main_align(egui::Align::Max),
        |ui| ui.add(value),
    )
}
/// A number the user drags or types into, in the recessed field: Plex Mono 11.5 at the
/// right, `NUMBER` points wide unless the text needs more.
pub(crate) fn drag_value(ui: &mut Ui, value: egui::DragValue<'_>) -> Response {
    field(
        ui,
        vec2(NUMBER, CONTROL),
        egui::Layout::left_to_right(egui::Align::Center).with_main_align(egui::Align::Max),
        |ui| ui.add(value),
    )
}

// ---------------------------------------------------------------------------------------------
// Slider with its value field.

/// What the app's `Slider` hands over (see `crate::slider`).
pub(crate) struct SliderArgs<'a> {
    /// Reads the value, or writes it first when given one.
    pub access: std::rc::Rc<dyn Fn(Option<f64>) -> f64 + 'a>,
    pub range: RangeInclusive<f64>,
    pub integral: bool,
    pub logarithmic: bool,
    pub smallest_positive: Option<f64>,
    pub suffix: String,
    pub fixed_decimals: Option<usize>,
    #[allow(clippy::type_complexity)]
    pub formatter: Option<Box<dyn Fn(f64, RangeInclusive<usize>) -> String + 'a>>,
    #[allow(clippy::type_complexity)]
    pub parser: Option<Box<dyn Fn(&str) -> Option<f64> + 'a>>,
}
/// The slider and its number field. Call it in a right-to-left layout: the field comes first
/// (it is as wide as its text needs) and the rail takes the room left of it. Like the stock
/// slider it only clamps what the user drags or types. Returns the rail's rect as well.
pub(crate) fn slider(ui: &mut Ui, a: SliderArgs<'_>) -> (Response, Rect) {
    let scale = Scale {
        logarithmic: a.logarithmic,
        smallest_positive: a
            .smallest_positive
            .unwrap_or(if a.integral { 1. } else { 1e-6 }),
        largest_finite: f64::INFINITY,
    };
    let decimals = a.fixed_decimals.or(a.integral.then_some(0));
    let before = (a.access)(None);
    // The field moves at the rail's rate: the value per point of travel where the thumb is.
    let travel = (ui.available_width() - NUMBER - 8.).max(24.);
    let arrows = ui.input(|i| {
        use egui::Key;
        i.num_presses(Key::ArrowUp)
            + i.num_presses(Key::ArrowDown)
            + i.num_presses(Key::ArrowLeft)
            + i.num_presses(Key::ArrowRight)
    });
    let speed = if a.integral && arrows > 0 {
        1.
    } else {
        let at = normalized_from_value(before, a.range.clone(), &scale);
        let step = 0.5 / travel as f64;
        value_from_normalized(at + step, a.range.clone(), &scale)
            - value_from_normalized(at - step, a.range.clone(), &scale)
    };
    let access = a.access.clone();
    let range = a.range.clone();
    // A typed value is rounded as a dragged one is (3.7 in a whole number is 4).
    let write = move |v: Option<f64>| {
        access(v.map(|v| match decimals {
            Some(n) => egui::emath::round_to_decimals(v, n),
            None => v,
        }))
    };
    let mut value = egui::DragValue::from_get_set(write)
        .speed(speed)
        .range(range)
        .clamp_existing_to_range(false)
        .suffix(a.suffix);
    if let Some(n) = decimals {
        value = value.fixed_decimals(n);
    }
    if let Some(f) = a.formatter {
        value = value.custom_formatter(f);
    }
    if let Some(p) = a.parser {
        value = value.custom_parser(p);
    }
    let number = drag_value(ui, value);
    let width = ui.available_width().max(24.);
    let (rect, rail) = ui.allocate_exact_size(vec2(width, CONTROL), Sense::drag());
    // The thumb's centre runs the whole groove, so at either end half of it hangs over.
    let run = rect.x_range();
    let value = (a.access)(None);
    let value_at = |x: f32| {
        let n = egui::emath::remap_clamp(x, run, 0. ..=1.) as f64;
        value_from_normalized(n, a.range.clone(), &scale)
    };
    let position = |v: f64| {
        egui::lerp(
            run,
            normalized_from_value(v, a.range.clone(), &scale) as f32,
        )
    };
    let rounded = |v: f64| match decimals {
        Some(n) => egui::emath::round_to_decimals(v, n),
        None => v,
    };
    let mut new = None;
    if let Some(pointer) = rail.interact_pointer_pos() {
        let aim = ui.input(|i| i.aim_radius());
        new = Some(egui::emath::smart_aim::best_in_range_f64(
            value_at(pointer.x - aim),
            value_at(pointer.x + aim),
        ));
    }
    if rail.has_focus() {
        ui.memory_mut(|m| {
            m.set_focus_lock_filter(
                rail.id,
                egui::EventFilter {
                    horizontal_arrows: true,
                    ..Default::default()
                },
            );
        });
        let steps = ui.input(|i| {
            use egui::Key;
            i.num_presses(Key::ArrowRight) as f32 - i.num_presses(Key::ArrowLeft) as f32
        });
        if steps != 0. {
            // As egui's `stepped_value`: a whole number steps by one, anything else by a
            // point of travel, and at least to the next value the rounding keeps.
            let mut stepped = if a.integral {
                (value + steps as f64).clamp(*a.range.start(), *a.range.end())
            } else {
                let to = position(value) + steps;
                egui::emath::smart_aim::best_in_range_f64(value_at(to - 0.49), value_at(to + 0.49))
            };
            if let Some(n) = decimals {
                let least = 1.001 / 10_f64.powi(n as i32);
                if stepped > value {
                    stepped = stepped.max(value + least);
                } else if stepped < value {
                    stepped = stepped.min(value - least);
                }
            }
            new = Some(stepped);
        }
    }
    if let Some(v) = new.map(rounded)
        && v != value
    {
        (a.access)(Some(v));
    }
    let value = (a.access)(None);
    rail.widget_info(|| egui::WidgetInfo::slider(ui.is_enabled(), value, ""));
    if ui.is_rect_visible(rect.expand(6.)) {
        let (hover, _) = hover_press(ui, &rail);
        let held = match FORCED.get() {
            Some(f) => f.press as u8 as f32,
            None => fade(ui, rail.id, "held", rail.dragged(), 0.06),
        };
        let y = rect.center().y;
        let groove = Rect::from_min_max(pos2(rect.left(), y - 3.), pos2(rect.right(), y + 3.));
        paint(ui, Look::Groove, Seg::Solo, groove, 1.);
        let x = position(value);
        if x > groove.left() + 1. {
            ui.painter().rect_filled(
                Rect::from_min_max(pos2(groove.left() + 1., y - 2.), pos2(x, y + 2.)),
                2.,
                mix(ACC, Color32::BLACK, 0.22),
            );
        }
        let thumb = Rect::from_center_size(pos2(x, y), vec2(10., 16.));
        paint(ui, Look::Thumb, Seg::Solo, thumb, 1.);
        paint(ui, Look::ThumbHover, Seg::Solo, thumb, hover);
        paint(ui, Look::ThumbPressed, Seg::Solo, thumb, held);
        if focused(&rail) {
            paint(ui, Look::Ring(3), Seg::Solo, thumb, 1.);
        }
    }
    // As egui's slider: the field's id stands for both while it has the focus.
    let mut both = if number.gained_focus() || number.has_focus() || number.lost_focus() {
        number.union(rail)
    } else {
        rail.union(number)
    };
    if (a.access)(None) != before {
        both.mark_changed();
    }
    (both, rect)
}

// The slider's scale, copied from egui 0.37 (`widgets/slider_core.rs`, MIT or Apache-2.0),
// which keeps these private: with the same mapping a value sits where egui's slider puts it.

struct Scale {
    logarithmic: bool,
    smallest_positive: f64,
    largest_finite: f64,
}
const INFINITY: f64 = f64::INFINITY;
/// A logarithmic slider that reaches infinity spans this many decades.
const INF_RANGE_MAGNITUDE: f64 = 10.;
fn value_from_normalized(normalized: f64, range: RangeInclusive<f64>, spec: &Scale) -> f64 {
    use egui::emath::{lerp, remap};
    let (min, max) = (*range.start(), *range.end());
    if min.is_nan() || max.is_nan() {
        f64::NAN
    } else if min == max {
        min
    } else if min > max {
        value_from_normalized(1. - normalized, max..=min, spec)
    } else if normalized <= 0. {
        min
    } else if normalized >= 1. {
        max
    } else if spec.logarithmic {
        if max <= 0. {
            -value_from_normalized(normalized, -min..=-max, spec)
        } else if 0. <= min {
            let (min_log, max_log) = range_log10(min, max, spec);
            10_f64.powf(lerp(min_log..=max_log, normalized))
        } else {
            let zero_cutoff = logarithmic_zero_cutoff(min, max);
            if normalized < zero_cutoff {
                value_from_normalized(
                    remap(normalized, 0. ..=zero_cutoff, 0. ..=1.),
                    min..=0.,
                    spec,
                )
            } else {
                value_from_normalized(
                    remap(normalized, zero_cutoff..=1., 0. ..=1.),
                    0. ..=max,
                    spec,
                )
            }
        }
    } else {
        lerp(range, normalized.clamp(0., 1.))
    }
}
fn normalized_from_value(value: f64, range: RangeInclusive<f64>, spec: &Scale) -> f64 {
    use egui::emath::{remap, remap_clamp};
    let (min, max) = (*range.start(), *range.end());
    if min.is_nan() || max.is_nan() {
        f64::NAN
    } else if min == max {
        0.5
    } else if min > max {
        1. - normalized_from_value(value, max..=min, spec)
    } else if value <= min {
        0.
    } else if value >= max {
        1.
    } else if spec.logarithmic {
        if max <= 0. {
            normalized_from_value(-value, -min..=-max, spec)
        } else if 0. <= min {
            let (min_log, max_log) = range_log10(min, max, spec);
            remap_clamp(value.log10(), min_log..=max_log, 0. ..=1.)
        } else {
            let zero_cutoff = logarithmic_zero_cutoff(min, max);
            if value < 0. {
                remap(
                    normalized_from_value(value, min..=0., spec),
                    0. ..=1.,
                    0. ..=zero_cutoff,
                )
            } else {
                remap(
                    normalized_from_value(value, 0. ..=max, spec),
                    0. ..=1.,
                    zero_cutoff..=1.,
                )
            }
        }
    } else {
        remap_clamp(value, range, 0. ..=1.)
    }
}
fn range_log10(min: f64, max: f64, spec: &Scale) -> (f64, f64) {
    if min == 0. && max == INFINITY {
        (spec.smallest_positive.log10(), INF_RANGE_MAGNITUDE)
    } else if min == 0. {
        if spec.smallest_positive < max {
            (spec.smallest_positive.log10(), max.log10())
        } else {
            (max.log10() - INF_RANGE_MAGNITUDE, max.log10())
        }
    } else if max == INFINITY {
        if min < spec.largest_finite {
            (min.log10(), spec.largest_finite.log10())
        } else {
            (min.log10(), min.log10() + INF_RANGE_MAGNITUDE)
        }
    } else {
        (min.log10(), max.log10())
    }
}
/// Where zero sits on a logarithmic slider that crosses it.
fn logarithmic_zero_cutoff(min: f64, max: f64) -> f64 {
    let min_magnitude = if min == -INFINITY {
        INF_RANGE_MAGNITUDE
    } else {
        min.abs().log10().abs()
    };
    let max_magnitude = if max == INFINITY {
        INF_RANGE_MAGNITUDE
    } else {
        max.log10().abs()
    };
    let cutoff = min_magnitude / (min_magnitude + max_magnitude);
    debug_assert!((0. ..=1.).contains(&cutoff));
    cutoff
}

// ---------------------------------------------------------------------------------------------
// Section band and tab title.

/// The name at the top of a property tab.
pub(crate) fn title(ui: &mut Ui, text: &str) {
    ui.add_space(4.);
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), CONTROL), Sense::hover());
    let text = line(ui, text, semibold(14.), TX, rect.width());
    ui.painter().galley(
        pos2(rect.left(), rect.center().y - text.size().y / 2.),
        text,
        TX,
    );
    ui.add_space(8.);
}
/// `title` with a control at the right end of its line (the Look tab's preset): the control
/// takes what it needs, the name the rest.
pub(crate) fn title_with(ui: &mut Ui, text: &str, add: impl FnOnce(&mut Ui)) {
    ui.add_space(4.);
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), CONTROL), Sense::hover());
    let mut right = ui.new_child(
        egui::UiBuilder::new()
            .max_rect(rect)
            .layout(egui::Layout::right_to_left(egui::Align::Center)),
    );
    add(&mut right);
    let room = right.min_rect().left() - rect.left() - 8.;
    let text = line(ui, text, semibold(14.), TX, room);
    ui.painter().galley(
        pos2(rect.left(), rect.center().y - text.size().y / 2.),
        text,
        TX,
    );
    ui.add_space(8.);
}
/// A collapsible section: a band from edge to edge with an arrow, the title in small caps and
/// at the right a preview of the values inside. Open or closed is remembered per title.
pub(crate) fn fold<R>(
    ui: &mut Ui,
    title: &str,
    preview: String,
    default_open: bool,
    add: impl FnOnce(&mut Ui) -> R,
) -> Option<R> {
    fold_with(ui, title, title, preview, default_open, add)
}
/// The id `fold_with` keeps its open state under, for a caller that sets that state.
pub(crate) fn fold_id(ui: &Ui, salt: impl std::hash::Hash + std::fmt::Debug) -> egui::Id {
    ui.make_persistent_id(("fold", salt))
}
/// `fold`, its open state kept under `salt` rather than the title (the same title in several
/// places, each remembered on its own).
pub(crate) fn fold_with<R>(
    ui: &mut Ui,
    salt: impl std::hash::Hash + std::fmt::Debug,
    title: &str,
    preview: String,
    default_open: bool,
    add: impl FnOnce(&mut Ui) -> R,
) -> Option<R> {
    use egui::collapsing_header::CollapsingState;
    let id = fold_id(ui, salt);
    let mut state = CollapsingState::load_with_default_open(ui.ctx(), id, default_open);
    let (rect, r) = ui.allocate_exact_size(vec2(ui.available_width(), 26.), Sense::click());
    let r = r.on_hover_cursor(egui::CursorIcon::PointingHand);
    if r.clicked() {
        state.toggle(ui);
    }
    r.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, ui.is_enabled(), title));
    let band = rect.expand2(vec2(BLEED.get(), 0.));
    let clip = ui.clip_rect();
    let clip = Rect::from_min_max(
        pos2(band.left(), band.top().max(clip.top())),
        pos2(band.right(), band.bottom().min(clip.bottom())),
    );
    if clip.is_positive() {
        let painter = ui.painter().with_clip_rect(clip);
        let (hover, press) = hover_press(ui, &r);
        let fill = blend(blend(BAND, BAND_HOVER, hover), BAND_PRESSED, press);
        painter.rect_filled(band, 0., fill);
        let row = |top: f32, color| {
            painter.rect_filled(
                Rect::from_min_max(pos2(band.left(), top), pos2(band.right(), top + 1.)),
                0.,
                color,
            );
        };
        row(band.top(), LIGHT_LINE);
        row(band.bottom() - 1., EDGE);
        if focused(&r) {
            painter.rect_stroke(
                band,
                0.,
                egui::Stroke::new(1.5, ACC),
                egui::StrokeKind::Inside,
            );
        }
        let turn = ui.ctx().animate_bool_with_time_and_easing(
            id.with("arrow"),
            state.is_open(),
            0.14,
            ease,
        );
        let centre = pos2(rect.left() + 8., rect.center().y);
        let turned: Vec<Pos2> = {
            let rot = egui::emath::Rot2::from_angle(turn * std::f32::consts::FRAC_PI_2);
            CHEVRON_RIGHT
                .iter()
                .map(|p| centre + rot * (vec2(p[0] - 12., p[1] - 12.) * (16. / 24.)))
                .collect()
        };
        painter.add(Shape::line(turned, egui::Stroke::new(1.2, FAINT)));
        let mut job = egui::text::LayoutJob::single_section(
            title.to_uppercase(),
            egui::TextFormat {
                font_id: semibold(11.),
                extra_letter_spacing: 0.66,
                color: TX,
                ..Default::default()
            },
        );
        job.wrap = egui::text::TextWrapping::truncate_at_width(rect.width() - 22.);
        let name = painter.layout_job(job);
        let name_right = rect.left() + 22. + name.size().x;
        painter.galley(
            pos2(rect.left() + 22., rect.center().y - name.size().y / 2.),
            name,
            TX,
        );
        if !preview.is_empty() {
            let text = painter.layout_no_wrap(preview, mono(11.), DIM);
            // Right-aligned; what does not fit beside the title is cut off at its left.
            let room = Rect::from_min_max(
                pos2(name_right + 12., clip.top()),
                pos2(rect.right(), clip.bottom()),
            );
            if room.is_positive() {
                painter.with_clip_rect(room).galley(
                    pos2(
                        rect.right() - text.size().x,
                        rect.center().y - text.size().y / 2.,
                    ),
                    text,
                    DIM,
                );
            }
        }
    }
    state
        .show_body_unindented(ui, |ui| {
            ui.spacing_mut().item_spacing.y = 4.;
            ui.add_space(7.);
            let inner = add(ui);
            // The last row's gap is already there: 9 points in all.
            ui.add_space(5.);
            inner
        })
        .map(|r| r.inner)
}

/// A disclosure inside a section ("More"): a flat row with the chevron and the name in the
/// section's type, and `add`'s rows below it in the section's own column. Open or closed is
/// remembered under `salt`.
pub(crate) fn disclosure<R>(
    ui: &mut Ui,
    salt: impl std::hash::Hash + std::fmt::Debug,
    text: &str,
    default_open: bool,
    add: impl FnOnce(&mut Ui) -> R,
) -> Option<R> {
    use egui::collapsing_header::CollapsingState;
    let id = ui.make_persistent_id(salt);
    let mut state = CollapsingState::load_with_default_open(ui.ctx(), id, default_open);
    let label = line(ui, text, sans(12.5), DIM, f32::INFINITY);
    let width = 16. + 2. + label.size().x.ceil();
    let (rect, r) = ui.allocate_exact_size(vec2(width, CONTROL), Sense::click());
    let r = r.on_hover_cursor(egui::CursorIcon::PointingHand);
    if r.clicked() {
        state.toggle(ui);
    }
    r.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, ui.is_enabled(), text));
    if ui.is_rect_visible(rect) {
        let (hover, _) = hover_press(ui, &r);
        let turn = ui.ctx().animate_bool_with_time_and_easing(
            id.with("arrow"),
            state.is_open(),
            0.14,
            ease,
        );
        let centre = pos2(rect.left() + 8., rect.center().y);
        stroke_icon(
            ui,
            centre,
            &CHEVRON_RIGHT,
            turn * std::f32::consts::FRAC_PI_2,
            1.2,
            blend(FAINT, TX, hover),
        );
        let color = blend(DIM, TX, hover);
        let label = line(ui, text, sans(12.5), color, f32::INFINITY);
        ui.painter().galley(
            pos2(rect.left() + 18., rect.center().y - label.size().y / 2.),
            label,
            color,
        );
        if focused(&r) {
            paint(ui, Look::Ring(4), Seg::Solo, rect, 1.);
        }
    }
    state
        .show_body_unindented(ui, |ui| {
            ui.add_space(2.);
            add(ui)
        })
        .map(|r| r.inner)
}

/// The chevron of a disclosure at `centre`, turned `open` (0 closed, 1 open) of the way down.
fn disclosure_chevron(ui: &Ui, centre: Pos2, open: f32, color: Color32) {
    stroke_icon(
        ui,
        centre,
        &CHEVRON_RIGHT,
        open * std::f32::consts::FRAC_PI_2,
        1.2,
        color,
    );
}
/// The disclosure chevron as a click target of its own, for a header the caller lays out (a
/// feature's checkbox and title beside it): toggles `state` and turns the way `disclosure`'s
/// does.
pub(crate) fn chevron_toggle(
    ui: &mut Ui,
    state: &mut egui::collapsing_header::CollapsingState,
) -> Response {
    let (rect, r) = ui.allocate_exact_size(vec2(16., CONTROL), Sense::click());
    let r = r.on_hover_cursor(egui::CursorIcon::PointingHand);
    if r.clicked() {
        state.toggle(ui);
    }
    r.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, ui.is_enabled(), "Expand"));
    if ui.is_rect_visible(rect) {
        let (hover, _) = hover_press(ui, &r);
        let turn = ui.ctx().animate_bool_with_time_and_easing(
            r.id.with("arrow"),
            state.is_open(),
            0.14,
            ease,
        );
        disclosure_chevron(
            ui,
            pos2(rect.left() + 8., rect.center().y),
            turn,
            blend(FAINT, TX, hover),
        );
        if focused(&r) {
            paint(ui, Look::Ring(4), Seg::Solo, rect, 1.);
        }
    }
    r
}
/// A row that folds a group of rows (the outliner's "Objects"): the chevron and the name in
/// the panel's type. `open` is the caller's state; a click is the caller's to act on.
pub(crate) fn group_row(ui: &mut Ui, open: bool, text: &str, color: Color32) -> Response {
    let label = line(ui, text, sans(12.5), color, f32::INFINITY);
    let (rect, r) =
        ui.allocate_exact_size(vec2(18. + label.size().x.ceil(), CONTROL), Sense::click());
    let r = r.on_hover_cursor(egui::CursorIcon::PointingHand);
    r.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, ui.is_enabled(), text));
    if ui.is_rect_visible(rect) {
        let (hover, _) = hover_press(ui, &r);
        let turn = ui
            .ctx()
            .animate_bool_with_time_and_easing(r.id.with("arrow"), open, 0.14, ease);
        disclosure_chevron(
            ui,
            pos2(rect.left() + 8., rect.center().y),
            turn,
            blend(FAINT, TX, hover),
        );
        let color = blend(color, Color32::WHITE, hover * 0.5);
        let label = line(ui, text, sans(12.5), color, f32::INFINITY);
        ui.painter().galley(
            pos2(rect.left() + 18., rect.center().y - label.size().y / 2.),
            label,
            color,
        );
        if focused(&r) {
            paint(ui, Look::Ring(4), Seg::Solo, rect, 1.);
        }
    }
    r
}
/// The header of a band that folds (the outliner's): the chevron and the title in small caps
/// as a fold's.
pub(crate) fn band_title(ui: &mut Ui, open: bool, text: &str) -> Response {
    let mut job = egui::text::LayoutJob::single_section(
        text.to_uppercase(),
        egui::TextFormat {
            font_id: semibold(11.),
            extra_letter_spacing: 0.66,
            color: TX,
            ..Default::default()
        },
    );
    job.wrap = egui::text::TextWrapping::truncate_at_width(f32::INFINITY);
    let name = ui.painter().layout_job(job);
    let (rect, r) = ui.allocate_exact_size(
        vec2(22. + name.size().x.ceil() + 4., CONTROL),
        Sense::click(),
    );
    let r = r.on_hover_cursor(egui::CursorIcon::PointingHand);
    r.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, ui.is_enabled(), text));
    if ui.is_rect_visible(rect) {
        let (hover, _) = hover_press(ui, &r);
        let turn = ui
            .ctx()
            .animate_bool_with_time_and_easing(r.id.with("arrow"), open, 0.14, ease);
        disclosure_chevron(
            ui,
            pos2(rect.left() + 8., rect.center().y),
            turn,
            blend(FAINT, TX, hover),
        );
        ui.painter().galley(
            pos2(rect.left() + 22., rect.center().y - name.size().y / 2.),
            name,
            TX,
        );
        if focused(&r) {
            paint(ui, Look::Ring(4), Seg::Solo, rect, 1.);
        }
    }
    r
}

// ---------------------------------------------------------------------------------------------
// The control sheet.

/// Every control in every state, as on the board's sheet (`--ui-skin-sheet`): the way to
/// compare the skin with the board, since a headless capture has no pointer to hover with.
pub(crate) fn sheet(ctx: &egui::Context) {
    let frame = egui::Frame::new().fill(PANEL).inner_margin(24.);
    let area = egui::Area::new(egui::Id::unique("skin sheet"))
        .anchor(egui::Align2::LEFT_TOP, [60., 50.])
        .order(egui::Order::Foreground);
    area.show(ctx, |ui| {
        frame.show(ui, |ui| scope(ui, 0., sheet_rows));
    });
}
fn sheet_rows(ui: &mut Ui) {
    let states = [
        ("Rest", Forced::default(), true),
        (
            "Hover",
            Forced {
                hover: true,
                ..Default::default()
            },
            true,
        ),
        (
            "Pressed",
            Forced {
                hover: true,
                press: true,
                ..Default::default()
            },
            true,
        ),
        (
            "Focused",
            Forced {
                focus: true,
                ..Default::default()
            },
            true,
        ),
        ("Disabled", Forced::default(), false),
    ];
    let caps = |text: &str| {
        egui::RichText::new(text.to_uppercase())
            .font(sans(11.))
            .color(FAINT)
    };
    type Draw<'a> = &'a dyn Fn(&mut Ui, usize);
    let value = Cell::new(0.4);
    let rows: [(&str, Draw); 13] = [
        ("Button", &|ui, _| {
            button(ui, "Duplicate");
        }),
        ("Flat button", &|ui, _| {
            flat(ui, false, "Save image");
        }),
        ("Primary button", &|ui, _| {
            primary(ui, "Export video");
        }),
        ("Toggle, on", &|ui, _| {
            toggle(ui, true, "Compare");
        }),
        ("Combo", &|ui, _| {
            Push {
                chevron: true,
                width: Some(140.),
                ..Push::new("Blackman-Harris")
            }
            .show(ui);
        }),
        ("Checkbox", &|ui, column| {
            checkbox_label(ui, &mut (column != 1), "Casts shadows");
        }),
        ("Radio", &|ui, column| {
            radio(ui, column != 1, "Perspective");
        }),
        ("Switch", &|ui, column| {
            switch(ui, &mut (column != 1));
        }),
        ("Slider and value", &|ui, column| {
            value.set(if column == 2 { 0.55 } else { 0.4 });
            let layout = egui::Layout::right_to_left(egui::Align::Center);
            ui.allocate_ui_with_layout(vec2(150., CONTROL), layout, |ui| {
                slider(
                    ui,
                    SliderArgs {
                        access: std::rc::Rc::new(|v| {
                            if let Some(v) = v {
                                value.set(v);
                            }
                            value.get()
                        }),
                        range: 0. ..=1.,
                        integral: false,
                        logarithmic: false,
                        smallest_positive: None,
                        suffix: String::new(),
                        fixed_decimals: Some(2),
                        formatter: None,
                        parser: None,
                    },
                )
            });
        }),
        ("Text field", &|ui, _| {
            let mut name = "Tall block".to_owned();
            let layout = egui::Layout::left_to_right(egui::Align::Center);
            field(ui, vec2(140., CONTROL), layout, |ui| {
                ui.add(
                    egui::TextEdit::singleline(&mut name)
                        .desired_width(126.)
                        .min_size(vec2(140., CONTROL))
                        .align(egui::Align2::LEFT_CENTER)
                        .margin(egui::Margin::symmetric(7, 0)),
                )
            });
        }),
        ("Number field", &|ui, column| {
            let mut number = 0.25;
            drag_value(
                ui,
                egui::DragValue::new(&mut number)
                    .speed(0.01)
                    .suffix(if column == 4 { " frames" } else { "" }),
            );
        }),
        ("Icon button", &|ui, column| {
            // What a bar's icon button wears: `button_chrome` under the icon.
            let (rect, r) = ui.allocate_exact_size(vec2(26., CONTROL), Sense::click());
            let down = button_chrome(ui, rect, &r, column == 3, column == 1);
            let centre = rect.center() + vec2(0., down);
            stroke_icon(ui, centre, &[[12., 5.], [12., 19.]], 0., 1.7, TX);
            stroke_icon(ui, centre, &[[5., 12.], [19., 12.]], 0., 1.7, TX);
        }),
        ("Section header", &|ui, _| {
            ui.allocate_ui(vec2(150., 26.), |ui| {
                fold(ui, "Film", "Grain 0.20".to_owned(), false, |_| {});
            });
        }),
    ];
    egui::Grid::new("skin sheet")
        .num_columns(6)
        .spacing([16., 10.])
        .min_col_width(150.)
        .show(ui, |ui| {
            ui.label("");
            for (name, _, _) in states {
                ui.label(caps(name));
            }
            ui.end_row();
            for (name, draw) in rows {
                ui.label(egui::RichText::new(name).color(DIM));
                for (column, (_, state, enabled)) in states.into_iter().enumerate() {
                    ui.push_id(column, |ui| {
                        ui.add_enabled_ui(enabled, |ui| forced(state, || draw(ui, column)));
                    });
                }
                ui.end_row();
            }
        });
    ui.add_space(26.);
    ui.horizontal_top(|ui| {
        ui.spacing_mut().item_spacing.x = 40.;
        ui.vertical(|ui| {
            ui.label(caps("Menu"));
            ui.add_space(10.);
            let hover = Forced {
                hover: true,
                ..Default::default()
            };
            ui.horizontal_top(|ui| {
                ui.spacing_mut().item_spacing.x = 6.;
                let down = egui::Layout::top_down(egui::Align::Min);
                ui.allocate_ui_with_layout(vec2(232., 196.), down, |ui| {
                    menu(ui, |ui| {
                        ui.set_width(224.);
                        item(ui, false, "Open scene…");
                        forced(hover, || item(ui, false, "Save scene"));
                        item(ui, true, "Save scene as…");
                        separator(ui);
                        // A submenu's row: an arrow, filled while its submenu is open.
                        item_row(ui, false, "Window", "", true, true);
                        item_row(ui, false, "Settings", "", true, false);
                        ui.add_enabled_ui(false, |ui| item(ui, false, "Restore last session"));
                        item_row(ui, false, "Generate scene…", "Ctrl+G", false, false);
                    });
                });
                // The submenu beside it.
                ui.allocate_ui_with_layout(vec2(150., 100.), down, |ui| {
                    ui.push_id("submenu", |ui| {
                        menu(ui, |ui| {
                            ui.set_width(142.);
                            item(ui, false, "Scenes");
                            item(ui, true, "Assets");
                            item(ui, false, "Statistics");
                        });
                    });
                });
            });
        });
        ui.vertical(|ui| {
            ui.label(caps("Segment"));
            ui.add_space(10.);
            let mut choice = 0;
            let options = [(0, "All"), (1, "Outdoor"), (2, "Indoor"), (3, "Studio")];
            ui.horizontal(|ui| segment(ui, &mut choice, &options));
        });
        ui.vertical(|ui| {
            ui.label(caps("Combo, open"));
            ui.add_space(10.);
            // A real one, held open: the popup as the tabs get it.
            let mut filter = 1;
            ui.allocate_ui(vec2(170., CONTROL), |ui| {
                let id = ui.next_auto_id().with("popup");
                egui::Popup::open_id(ui.ctx(), id);
                combo(ui, "Blackman-Harris", |ui| {
                    for (i, name) in ["Box", "Blackman-Harris", "Gaussian"].iter().enumerate() {
                        item_value(ui, &mut filter, i, *name);
                    }
                });
            });
        });
        ui.vertical(|ui| {
            ui.spacing_mut().item_spacing.y = 8.;
            ui.label(caps("Type"));
            ui.label(egui::RichText::new("Title 14 semibold").font(semibold(14.)));
            ui.label("Body 12.5 regular");
            ui.label(egui::RichText::new("SECTION 11 CAPS").font(semibold(11.)));
            ui.label(egui::RichText::new("Numbers 11.5 mono 0123456789").font(mono(11.5)));
        });
    });
    ui.add_space(26.);
    ui.horizontal_top(|ui| {
        ui.spacing_mut().item_spacing.x = 40.;
        ui.vertical(|ui| {
            ui.label(caps("Menu button, open"));
            ui.add_space(10.);
            // A real one, held open: the popup the bars get, with a submenu's row.
            ui.allocate_ui(vec2(120., CONTROL), |ui| {
                let id = ui.next_auto_id().with("popup");
                egui::Popup::open_id(ui.ctx(), id);
                menu_button_raised(ui, "Edit", |ui| {
                    ui.set_min_width(150.);
                    item(ui, false, "Undo");
                    item(ui, false, "Redo");
                    separator(ui);
                    menu_button(ui, "Window", |ui| {
                        item(ui, false, "Scenes");
                    });
                });
            });
        });
        ui.vertical(|ui| {
            ui.spacing_mut().item_spacing.y = 10.;
            ui.label(caps("Tooltip"));
            // As egui draws one: the frame from the global style.
            egui::Frame::popup(ui.style()).show(ui, |ui| {
                ui.label("Double-click the slider to reset.");
            });
            ui.label(caps("Window"));
            window_frame().show(ui, |ui| {
                ui.set_width(170.);
                ui.label(egui::RichText::new("Statistics").font(semibold(14.)));
                ui.label("Camera paths traced");
            });
        });
        ui.vertical(|ui| {
            ui.label(caps("Colour picker"));
            ui.add_space(10.);
            let mut hsva = egui::ecolor::Hsva::new(0.55, 0.6, 0.8, 1.);
            egui::Frame::NONE.inner_margin(4.).show(ui, |ui| {
                menu_content(ui, |ui| {
                    ui.spacing_mut().item_spacing = vec2(8., 6.);
                    ui.add_space(4.);
                    picker(ui, &mut hsva);
                    ui.add_space(4.);
                })
            });
        });
    });
}

// ---------------------------------------------------------------------------------------------
// The frame around the viewport: top bar, tool rail, tab strip, status bar.

/// The status bar's and the tab strip's ground.
pub(crate) const DEEP: Color32 = Color32::from_rgb(0x1a, 0x1a, 0x1c);
/// A tab under the pointer.
pub(crate) const TAB_HOVER: Color32 = Color32::from_rgb(0x24, 0x24, 0x27);
/// The dark line between regions.
pub(crate) const EDGE_LINE: Color32 = EDGE;

/// A vertical two-colour gradient over `rect`.
pub(crate) fn gradient(painter: &egui::Painter, rect: Rect, top: Color32, bottom: Color32) {
    let mut mesh = egui::Mesh::default();
    let white = egui::epaint::WHITE_UV;
    for (p, c) in [
        (rect.left_top(), top),
        (rect.right_top(), top),
        (rect.right_bottom(), bottom),
        (rect.left_bottom(), bottom),
    ] {
        mesh.vertices.push(egui::epaint::Vertex {
            pos: p,
            uv: white,
            color: c,
        });
    }
    mesh.indices.extend([0, 1, 2, 0, 2, 3]);
    painter.add(Shape::mesh(mesh));
}
/// The top bar's ground (`.top`): the gradient, the highlight under its top edge, and the
/// dark line along its bottom, inside `rect`.
pub(crate) fn top_bar_ground(painter: &egui::Painter, rect: Rect) {
    gradient(
        painter,
        rect,
        Color32::from_rgb(0x2c, 0x2c, 0x30),
        Color32::from_rgb(0x24, 0x24, 0x27),
    );
    painter.hline(
        rect.x_range(),
        rect.top() + 0.5,
        egui::Stroke::new(1_f32, Color32::from_white_alpha(20)),
    );
    painter.hline(
        rect.x_range(),
        rect.bottom() - 0.5,
        egui::Stroke::new(1_f32, EDGE),
    );
}
/// A section band's ground (`.outl`, a fold's header): the band colour, the highlight at the
/// top, the dark line at the bottom.
pub(crate) fn band_ground(painter: &egui::Painter, rect: Rect) {
    painter.rect_filled(rect, 0., BAND);
    painter.hline(
        rect.x_range(),
        rect.top() + 0.5,
        egui::Stroke::new(1_f32, Color32::from_white_alpha(20)),
    );
    painter.hline(
        rect.x_range(),
        rect.bottom() - 0.5,
        egui::Stroke::new(1_f32, EDGE),
    );
}
/// A group separator in a row of controls (`.vr`): a dark line beside a light one, 18 points
/// tall with 4 points of margin each side.
pub(crate) fn rule_v(ui: &mut Ui, height: f32) {
    let (rect, _) = ui.allocate_exact_size(vec2(2. + 8., height), Sense::hover());
    let x = rect.center().x;
    let y = rect.center().y;
    let span = y - 9.0..=y + 9.0;
    ui.painter()
        .vline(x - 0.5, span.clone(), egui::Stroke::new(1_f32, EDGE));
    ui.painter()
        .vline(x + 0.5, span, egui::Stroke::new(1_f32, LIGHT_LINE));
}
/// The same across a column (the tool rail's), `width` points wide.
pub(crate) fn rule_h(ui: &mut Ui, width: f32) {
    let (rect, _) = ui.allocate_exact_size(vec2(width, 2. + 12.), Sense::hover());
    let y = rect.center().y;
    let span = rect.center().x - 10.0..=rect.center().x + 10.0;
    ui.painter()
        .hline(span.clone(), y - 0.5, egui::Stroke::new(1_f32, EDGE));
    ui.painter()
        .hline(span, y + 0.5, egui::Stroke::new(1_f32, LIGHT_LINE));
}
/// The chrome of one button in a joined set (an icon button the caller draws itself):
/// like `button_chrome`, raised, with `seg` saying which corners are round. Returns how far
/// down the content sits.
pub(crate) fn segment_chrome(ui: &Ui, rect: Rect, response: &Response, seg: Seg, on: bool) -> f32 {
    0.5 * paint_push(ui, rect, response, Chrome::Raised, on, false, seg)
}
/// The shadow under a whole joined set, `rect` being the set's box.
pub(crate) fn segment_shadow(ui: &Ui, rect: Rect) -> Shape {
    chrome(ui.ctx(), Look::SegmentShadow, Seg::Solo, rect, 1.)
}
/// A tab of the tab strip (`.tabs .b`): the strip's ground, or the panel's with the accent
/// bar while open, the lighter ground under the pointer.
pub(crate) fn tab_chrome(
    ui: &Ui,
    rect: Rect,
    response: &Response,
    open: bool,
    across: bool,
) -> (f32, Color32) {
    let (hover, press) = hover_press(ui, response);
    let painter = ui.painter();
    if open {
        painter.rect_filled(rect, 0., PANEL);
        let stroke = egui::Stroke::new(1_f32, EDGE);
        if across {
            painter.rect_filled(
                Rect::from_min_size(rect.min, vec2(rect.width(), 2.)),
                0.,
                ACC,
            );
            painter.vline(rect.right() - 0.5, rect.y_range(), stroke);
        } else {
            painter.rect_filled(
                Rect::from_min_size(rect.min, vec2(2., rect.height())),
                0.,
                ACC,
            );
            painter.hline(rect.x_range(), rect.bottom() - 0.5, stroke);
        }
    } else if hover > 0. {
        painter.rect_filled(rect, 0., TAB_HOVER.gamma_multiply(hover));
    }
    if focused(response) {
        paint(ui, Look::Ring(0), Seg::Solo, rect.shrink(1.5), 1.);
    }
    let color = if open { ACC } else { blend(FAINT, TX, hover) };
    (0.5 * press, color)
}

/// A read-only number in a recessed field (the top bar's sample counter): mono text in the
/// dim colour, the field's chrome behind it.
pub(crate) fn readout(ui: &mut Ui, text: &str) -> Response {
    let galley = line(ui, text, mono(11.5), DIM, f32::INFINITY);
    let width = (galley.size().x + 16.).ceil();
    let (rect, r) = ui.allocate_exact_size(vec2(width, CONTROL), Sense::hover());
    if ui.is_rect_visible(rect) {
        paint(ui, Look::Field, Seg::Solo, rect, 1.);
        ui.painter().galley(
            pos2(rect.left() + 8., rect.center().y - galley.size().y / 2.),
            galley,
            DIM,
        );
    }
    r
}
/// The width `readout` takes for `text`.
pub(crate) fn readout_width(ui: &Ui, text: &str) -> f32 {
    (line(ui, text, mono(11.5), DIM, f32::INFINITY).size().x + 16.).ceil()
}
/// A progress bar in the status bar: the recess of a slider's rail with the accent filling it.
pub(crate) fn progress(ui: &mut Ui, fraction: f32, width: f32) -> Response {
    let (rect, r) = ui.allocate_exact_size(vec2(width, 6.), Sense::hover());
    if ui.is_rect_visible(rect) {
        paint(ui, Look::Groove, Seg::Solo, rect, 1.);
        let fill = Rect::from_min_size(
            rect.min + vec2(1., 1.),
            vec2(
                (rect.width() - 2.) * fraction.clamp(0., 1.),
                rect.height() - 2.,
            ),
        );
        if fill.width() > 0.5 {
            ui.painter()
                .rect_filled(fill, 2., mix(ACC, Color32::BLACK, 0.22));
        }
    }
    r
}

#[cfg(test)]
mod tests {
    use super::*;
    /// A raised button, pixel by pixel: the edge, the highlight under it, the gradient's two
    /// ends, and the drop shadow below.
    #[test]
    fn a_raised_button_is_drawn_as_the_board_says() {
        let s = spec(Look::Raised, Seg::Solo);
        assert_eq!(s.pad(), 3.);
        let pad = 3;
        let (w, h) = (21, 24);
        let image = raster(&s, w, h, pad, 1., false);
        let at = |x: usize, y: usize| image[(y + pad) * (w + 2 * pad) + x + pad];
        // The border, away from the corners.
        assert_eq!(at(10, 0), EDGE);
        assert_eq!(at(0, 12), EDGE);
        assert_eq!(at(10, 23), EDGE);
        // The first fill row carries the 8 % highlight on #3b3c41.
        let top = at(10, 1);
        assert!(top.r() > 0x3b + 12 && top.r() < 0x3b + 20, "{top:?}");
        // The gradient ends at #2f3034.
        let bottom = at(10, 22);
        assert!((bottom.r() as i32 - 0x2f).abs() <= 1, "{bottom:?}");
        assert!((bottom.b() as i32 - 0x34).abs() <= 1, "{bottom:?}");
        // The corner is cut, and the shadow shows under the bottom edge only.
        assert_eq!(image[pad * (w + 2 * pad) + pad].a(), 0);
        let under = at(10, 24);
        assert!(under.a() > 40 && under.r() == 0, "{under:?}");
        assert_eq!(image[w / 2 + pad].a(), 0);
    }
    /// A pressed button has no highlight and darkens towards its top edge.
    #[test]
    fn a_pressed_button_is_shaded_from_the_top() {
        let image = raster(&spec(Look::Pressed, Seg::Solo), 21, 24, 0, 1., false);
        let at = |y: usize| image[y * 21 + 10];
        assert!(at(1).r() < at(4).r() && at(4).r() < at(12).r());
        assert!(at(1).r() < 0x27);
    }
    /// The same box at 150 %: half as many pixels again, and the same colours in the middle.
    #[test]
    fn the_scale_changes_the_pixels_not_the_look() {
        let s = spec(Look::Field, Seg::Solo);
        let one = raster(&s, 20, 24, 0, 1., false);
        let more = raster(&s, 30, 36, 0, 1.5, false);
        assert_eq!(one[12 * 20 + 10], more[18 * 30 + 15]);
        assert_eq!(one[12 * 20 + 10], Color32::from_rgb(0x14, 0x14, 0x16));
    }
    /// In linear light the light line under a field and a button's drop shadow land on the
    /// panel where code-value blending puts them.
    #[test]
    fn linear_light_keeps_the_translucent_pixels() {
        let panel = rgba(PANEL);
        for look in [Look::Field, Look::Raised] {
            let s = spec(look, Seg::Solo);
            let pad = s.pad() as usize;
            // The row under the box, in its middle.
            let at = |image: &[Color32]| {
                let c = image[(24 + pad) * (21 + 2 * pad) + 10 + pad];
                [c.r(), c.g(), c.b(), c.a()].map(|v| v as f32 / 255.)
            };
            let plain = at(&raster(&s, 21, 24, pad, 1., false));
            let fixed = at(&raster(&s, 21, 24, pad, 1., true));
            assert!(plain[3] > 0.02 && plain[3] < 0.98);
            for k in 0..3 {
                let wanted = plain[k] + panel[k] * (1. - plain[3]);
                // gui.hlsl: linearise the straight colour, premultiply, blend.
                let got = to_linear(fixed[k] / fixed[3]) * fixed[3]
                    + to_linear(panel[k]) * (1. - fixed[3]);
                assert!((to_linear(wanted) - got).abs() < 0.004, "{look:?} {k}");
            }
        }
    }
    /// A checkbox's focus outline is a line 1.5 points off the box, not a band against it.
    #[test]
    fn an_outline_leaves_its_gap() {
        let s = spec(Look::Outline(3), Seg::Solo);
        let pad = s.pad() as usize;
        // At 200 %: the gap is 3 pixels, the line the 3 after it.
        let image = raster(&s, 30, 30, 2 * pad, 2., false);
        let above = |pixels: usize| image[(2 * pad - pixels) * (30 + 4 * pad) + 2 * pad + 15];
        assert_eq!(above(1).a(), 0);
        assert_eq!(above(3).a(), 0);
        assert_eq!(above(5), ACC);
    }
    /// A full atlas is kept for the frame that filled it and starts over larger at the next.
    #[test]
    fn a_full_atlas_grows_at_the_next_frame() {
        let mut a = Atlas::new();
        let one = a
            .entry((Look::Raised, Seg::Solo, 0, 24), 1., false)
            .unwrap();
        a.full = true;
        assert!(
            a.entry((Look::Pressed, Seg::Solo, 0, 24), 1., false)
                .is_none()
        );
        assert_eq!(
            a.entry((Look::Raised, Seg::Solo, 0, 24), 1., false)
                .unwrap()
                .at,
            one.at
        );
        a.grow = true;
        assert!(
            a.entry((Look::Pressed, Seg::Solo, 0, 24), 1., false)
                .is_some()
        );
        assert_eq!((a.side, a.entries.len(), a.full), (1024, 1, false));
    }
    #[test]
    fn the_eased_curve_runs_from_zero_to_one() {
        assert!(ease(0.).abs() < 1e-3 && (ease(1.) - 1.).abs() < 1e-3);
        // cubic-bezier(.3, .7, .3, 1) is ahead of linear all the way.
        for t in [0.2, 0.5, 0.8] {
            assert!(ease(t) > t);
        }
    }
    /// The copied scale agrees with itself, on the ranges the tabs use.
    #[test]
    fn the_slider_scale_round_trips() {
        let log = Scale {
            logarithmic: true,
            smallest_positive: 1e-6,
            largest_finite: f64::INFINITY,
        };
        for (range, v) in [(1. ..=1e6, 4096.), (0.0005..=0.2, 0.01), (0. ..=100., 3.)] {
            let n = normalized_from_value(v, range.clone(), &log);
            assert!((value_from_normalized(n, range, &log) - v).abs() < 1e-9 * v.max(1.));
        }
        let linear = Scale {
            logarithmic: false,
            ..log
        };
        assert_eq!(normalized_from_value(0.25, 0. ..=1., &linear), 0.25);
        assert_eq!(value_from_normalized(0.5, 1. ..=64., &linear), 32.5);
    }
    /// The atlas hands out separate places and starts over when the scale changes.
    #[test]
    fn the_atlas_packs_and_resets() {
        let mut a = Atlas::new();
        let one = a
            .entry((Look::Raised, Seg::Solo, 0, 24), 1., false)
            .unwrap();
        let two = a
            .entry((Look::Pressed, Seg::Solo, 0, 24), 1., false)
            .unwrap();
        assert!(two.at[0] > one.at[0] + one.size[0] || two.at[1] > one.at[1]);
        assert_eq!(one.cap[1], 0);
        assert!(one.cap[0] > one.pad);
        let again = a
            .entry((Look::Raised, Seg::Solo, 0, 24), 1., false)
            .unwrap();
        assert_eq!(again.at, one.at);
        let popup = a.entry((Look::Popup, Seg::Solo, 0, 0), 1., false).unwrap();
        assert!(popup.cap[0] > 0 && popup.cap[1] > 0);
        a.entry((Look::Raised, Seg::Solo, 0, 36), 1.5, false)
            .unwrap();
        assert_eq!(a.entries.len(), 1);
    }
}
