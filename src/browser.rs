//! The Assets window (roadmap item 6): browse Poly Haven, ambientCG and the Khronos glTF
//! samples, download at a chosen resolution, and use the result in one click (HDRI, texture
//! set on the selected object / floor / walls, or a model added to the scene). Catalogs,
//! downloads and thumbnails all run on background threads (`assets.rs`).
use crate::assets::{self, Asset, Job, Kind, Source};
use crate::ui::{ACCENT, Selection, field_hint, note, placeholder};
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::mpsc::{Receiver, Sender, channel};
use std::time::Duration;

const PAGE: usize = 48;
type CatalogResult = Result<Vec<Asset>, String>;
/// Assets found in the scenes folders, each with the resolutions it is there at.
type Local = Vec<(Asset, Vec<String>)>;
/// Poly Haven's HDRI categories offered as a filter (tag, label). The catalog keeps categories
/// in front of the tags; every HDRI carries at least one of these three.
const HDRI_CATEGORIES: [(&str, &str); 3] = [
    ("outdoor", "Outdoor"),
    ("indoor", "Indoor"),
    ("studio", "Studio"),
];
/// Running downloads the strip lists before it says "+ N more".
const STRIP_ROWS: usize = 3;

pub struct Browser {
    pub open: bool,
    /// Catalog tabs, or the list of other sources.
    others: bool,
    /// The Downloaded list (what is in the scenes folders, across sources) instead of a catalog.
    local: bool,
    /// The Downloaded list's kind filter (None: all kinds).
    local_kind: Option<Kind>,
    source: Source,
    kind: Kind,
    /// HDRIs only: a tag of `HDRI_CATEGORIES`, or empty for all.
    category: &'static str,
    search: String,
    page: usize,
    selected: Option<Asset>,
    /// Chosen resolution per (source, kind).
    res: HashMap<(Source, Kind), String>,
    catalogs: HashMap<Source, Vec<Asset>>,
    loading: HashMap<Source, Receiver<CatalogResult>>,
    status: String,
    thumbs: HashMap<String, egui::TextureHandle>,
    thumb_requested: HashSet<String>,
    thumb_tx: Option<Sender<Asset>>,
    thumb_rx: Option<Receiver<(String, Result<PathBuf, String>)>>,
    thumb_ready: Vec<(String, PathBuf)>,
    /// Thumbnails that failed (no preview, offline): counted so the window stops waiting.
    thumb_failed: usize,
    jobs: Vec<Job>,
    /// Failed downloads (name, error): shown in the strip until dismissed or the next download.
    failed: Vec<(String, String)>,
    installed: Local,
    /// A running scan of the scenes folders; `scanned` is false when one is due.
    scan: Option<Receiver<Local>>,
    scanned: bool,
    /// Self-test: a catalog to show without the network.
    pub offline: bool,
}
impl Default for Browser {
    fn default() -> Self {
        Self {
            open: false,
            others: false,
            local: false,
            local_kind: None,
            source: Source::PolyHaven,
            kind: Kind::Hdri,
            category: "",
            search: String::new(),
            page: 0,
            selected: None,
            res: HashMap::new(),
            catalogs: HashMap::new(),
            loading: HashMap::new(),
            status: String::new(),
            thumbs: HashMap::new(),
            thumb_requested: HashSet::new(),
            thumb_tx: None,
            thumb_rx: None,
            thumb_ready: vec![],
            thumb_failed: 0,
            jobs: vec![],
            failed: vec![],
            installed: vec![],
            scan: None,
            scanned: false,
            offline: false,
        }
    }
}
fn key(a: &Asset) -> String {
    format!("{:?}/{:?}/{}", a.source, a.kind, a.id)
}
/// The default resolution: 2k (4k for HDRIs, which are seen as backgrounds), else the
/// largest offered below it.
fn default_res(a: &Asset) -> String {
    let want = if a.kind == Kind::Hdri { "4k" } else { "2k" };
    a.resolutions
        .iter()
        .find(|r| r.eq_ignore_ascii_case(want))
        .or(a.resolutions.first())
        .cloned()
        .unwrap_or_default()
}
impl Browser {
    /// A catalog for tests and the self-test, so the window draws without the network.
    pub fn set_catalog(&mut self, source: Source, assets: Vec<Asset>) {
        self.selected = assets.first().cloned();
        self.source = source;
        self.kind = assets.first().map_or(Kind::Hdri, |a| a.kind);
        self.catalogs.insert(source, assets);
        self.scanned = false;
    }
    fn catalog(&mut self, source: Source) -> Option<&Vec<Asset>> {
        if !self.catalogs.contains_key(&source) && !self.loading.contains_key(&source) {
            if let Some(cached) = assets::cached_catalog(source) {
                self.catalogs.insert(source, cached);
            } else if !self.offline {
                self.refresh(source);
            }
        }
        self.catalogs.get(&source)
    }
    fn refresh(&mut self, source: Source) {
        let (tx, rx) = channel();
        std::thread::spawn(move || {
            let _ = tx.send(assets::fetch_catalog(source).map_err(|e| format!("{e:#}")));
        });
        self.loading.insert(source, rx);
    }
    /// Looks for every catalog entry in the scenes folders, on a thread: some 15 000 paths.
    /// Catalogs come from memory or the disk cache, never the network, so this works offline;
    /// a source whose catalog was never fetched contributes nothing.
    fn start_scan(&mut self) {
        let mut catalogs = self.catalogs.clone();
        let (tx, rx) = channel();
        std::thread::spawn(move || {
            let roots = crate::dirs::scene_roots();
            let mut out: Local = vec![];
            for source in Source::ALL {
                let catalog = catalogs
                    .remove(&source)
                    .or_else(|| assets::cached_catalog(source));
                for a in catalog.unwrap_or_default() {
                    let have = |res: &str| {
                        let rel = assets::use_path(&a, res);
                        roots.iter().any(|r| r.join(&rel).is_file())
                    };
                    let found: Vec<String> = if a.resolutions.is_empty() {
                        have("").then(String::new).into_iter().collect()
                    } else {
                        a.resolutions.iter().filter(|r| have(r)).cloned().collect()
                    };
                    if !found.is_empty() {
                        out.push((a, found));
                    }
                }
            }
            let _ = tx.send(out);
        });
        self.scan = Some(rx);
        self.scanned = true;
    }
    /// Four threads fetch thumbnails as cards are drawn.
    fn request_thumbnail(&mut self, a: &Asset) {
        if self.offline || !self.thumb_requested.insert(key(a)) {
            return;
        }
        if self.thumb_tx.is_none() {
            let (tx, rx) = channel::<Asset>();
            let (done_tx, done_rx) = channel();
            let rx = std::sync::Arc::new(std::sync::Mutex::new(rx));
            for _ in 0..4 {
                let (rx, done_tx) = (rx.clone(), done_tx.clone());
                std::thread::spawn(move || {
                    loop {
                        let next = rx.lock().unwrap_or_else(|e| e.into_inner()).recv();
                        let Ok(a) = next else { break };
                        let result = assets::fetch_thumbnail(&a).map_err(|e| format!("{e:#}"));
                        if done_tx.send((key(&a), result)).is_err() {
                            break;
                        }
                    }
                });
            }
            self.thumb_tx = Some(tx);
            self.thumb_rx = Some(done_rx);
        }
        if let Some(tx) = &self.thumb_tx {
            let _ = tx.send(a.clone());
        }
    }
    /// Collects catalogs, thumbnails and finished downloads. Returns the finished downloads.
    fn poll(&mut self, ctx: &egui::Context) -> Vec<(Asset, Result<String, String>)> {
        let mut busy = false;
        let sources: Vec<Source> = self.loading.keys().copied().collect();
        for source in sources {
            match self.loading[&source].try_recv() {
                Ok(Ok(catalog)) => {
                    self.status = format!("{}: {} assets", source.label(), catalog.len());
                    self.catalogs.insert(source, catalog);
                    self.loading.remove(&source);
                    self.scanned = false;
                }
                Ok(Err(e)) => {
                    self.status = format!("{}: {e}", source.label());
                    self.loading.remove(&source);
                }
                Err(_) => busy = true,
            }
        }
        if let Some(rx) = &self.thumb_rx {
            while let Ok((k, result)) = rx.try_recv() {
                match result {
                    Ok(path) => self.thumb_ready.push((k, path)),
                    Err(_) => self.thumb_failed += 1,
                }
            }
        }
        // A few decodes per frame keep scrolling smooth.
        for (k, path) in self.thumb_ready.drain(..self.thumb_ready.len().min(6)) {
            if let Ok(img) = std::fs::read(&path)
                .map_err(anyhow::Error::from)
                .and_then(|b| Ok(image::load_from_memory(&b)?))
            {
                let img = img.thumbnail(256, 256).to_rgba8();
                let size = [img.width() as usize, img.height() as usize];
                let image = egui::ColorImage::from_rgba_unmultiplied(size, img.as_raw());
                let texture = ctx.load_texture(&k, image, egui::TextureOptions::LINEAR);
                self.thumbs.insert(k, texture);
            }
        }
        busy |= !self.thumb_ready.is_empty()
            || self.thumb_requested.len() > self.thumbs.len() + self.thumb_failed;
        if let Some(rx) = &self.scan {
            match rx.try_recv() {
                Ok(found) => {
                    self.installed = found;
                    self.scan = None;
                }
                Err(std::sync::mpsc::TryRecvError::Empty) => busy = true,
                Err(_) => self.scan = None,
            }
        }
        if !self.scanned && self.scan.is_none() {
            self.start_scan();
            busy = true;
        }
        let mut finished = vec![];
        self.jobs.retain(|job| {
            let done = job.result.lock().unwrap_or_else(|e| e.into_inner()).take();
            match done {
                Some(result) => {
                    finished.push((job.asset.clone(), result));
                    false
                }
                None => true,
            }
        });
        // A new file is in the scenes folder: the Downloaded list is stale.
        if finished.iter().any(|(_, result)| result.is_ok()) {
            self.scanned = false;
        }
        if busy || !self.jobs.is_empty() {
            ctx.request_repaint_after(Duration::from_millis(150));
        }
        finished
    }
    /// The foot of the details pane: one row per running download (name, bar, size, Cancel)
    /// and the failures, which stay until dismissed or the next download.
    fn download_strip(&mut self, ui: &mut egui::Ui) {
        use std::sync::atomic::Ordering::Relaxed;
        if !self.jobs.is_empty() {
            let title = match self.jobs.len() {
                1 => "DOWNLOADING".to_string(),
                n => format!("DOWNLOADING {n}"),
            };
            crate::ui::skin::heading(ui, &title);
        }
        for job in self.jobs.iter().take(STRIP_ROWS) {
            let name = if job.res.is_empty() {
                job.asset.name.clone()
            } else {
                format!("{} · {}", job.asset.name, job.res)
            };
            ui.add(egui::Label::new(egui::RichText::new(name).small()).truncate());
            let done = job.progress.done.load(Relaxed);
            let total = job.progress.total.load(Relaxed);
            let cancelling = job.progress.cancel.load(Relaxed);
            // The total grows as each file of a set starts, and is 0 until the first answers.
            let (fraction, size) = if total > 0 {
                (
                    (done as f32 / total as f32).min(1.),
                    format!("{:.1} / {:.1} MB", done as f64 / 1e6, total as f64 / 1e6),
                )
            } else {
                (0., format!("{:.1} MB", done as f64 / 1e6))
            };
            ui.horizontal(|ui| {
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    // The thread sees the flag at its next read, so the row can linger a moment.
                    let label = if cancelling {
                        "Cancelling…"
                    } else {
                        "Cancel"
                    };
                    if ui
                        .add_enabled(!cancelling, crate::ui::skin::Button::new(label))
                        .clicked()
                    {
                        job.progress.cancel.store(true, Relaxed);
                    }
                    crate::ui::skin::bar(ui, fraction, ui.available_width(), 16., &size);
                });
            });
        }
        if self.jobs.len() > STRIP_ROWS {
            note(ui, &format!("+ {} more", self.jobs.len() - STRIP_ROWS));
        }
        if !self.failed.is_empty() {
            let color = ui.visuals().error_fg_color;
            for (name, error) in &self.failed {
                let text = egui::RichText::new(format!("{name}: {error}"));
                ui.add(egui::Label::new(text.small().color(color)).wrap());
            }
            if crate::ui::skin::small_button(ui, "Dismiss").clicked() {
                self.failed.clear();
            }
        }
    }
}
/// A thumbnail card: the board's card with the image (or initial) in it, the name under it, and
/// a green dot once downloaded.
fn card(
    ui: &mut egui::Ui,
    a: &Asset,
    texture: Option<&egui::TextureHandle>,
    selected: bool,
    installed: bool,
) -> egui::Response {
    use crate::ui::skin;
    const PAD: f32 = 4.;
    let size = egui::vec2(128., 128.);
    let (rect, r) = ui.allocate_exact_size(
        egui::vec2(size.x + 2. * PAD, size.y + 2. * PAD + 22.),
        egui::Sense::click(),
    );
    skin::card_face(ui, &r, rect, selected);
    let image = egui::Rect::from_min_size(rect.min + egui::vec2(PAD, PAD), size);
    // The well the picture sits in: the field's colour.
    ui.painter()
        .rect_filled(image, 3., egui::Color32::from_rgb(0x14, 0x14, 0x16));
    match texture {
        Some(t) => {
            // Fit inside the square, keeping the aspect ratio (Khronos screenshots are wide).
            let [w, h] = t.size().map(|v| v as f32);
            let scale = (size.x / w).min(size.y / h);
            let fit = egui::Rect::from_center_size(image.center(), egui::vec2(w, h) * scale);
            egui::Image::new((t.id(), fit.size()))
                .corner_radius(3.)
                .paint_at(ui, fit);
        }
        None => {
            ui.painter().text(
                image.center(),
                egui::Align2::CENTER_CENTER,
                a.name.chars().next().unwrap_or('?'),
                skin::semibold(40.),
                skin::FAINT,
            );
        }
    }
    if installed {
        ui.painter().circle_filled(
            image.right_top() + egui::vec2(-9., 9.),
            5.,
            egui::Color32::from_rgb(90, 200, 110),
        );
    }
    let mut job = egui::text::LayoutJob::single_section(
        a.name.clone(),
        egui::TextFormat {
            font_id: skin::sans(12.5),
            color: skin::TX,
            ..Default::default()
        },
    );
    job.wrap = egui::text::TextWrapping::truncate_at_width(size.x - 2.);
    let name = ui.painter().layout_job(job);
    ui.painter().galley(
        egui::pos2(image.min.x + 1., image.max.y + 5.),
        name,
        skin::TX,
    );
    r.on_hover_text(&a.name)
}
/// What to do with a downloaded asset.
enum Use {
    Hdri(String),
    Texture(String, Target),
    Model(String),
}
#[derive(Clone, Copy)]
enum Target {
    Object(usize),
    Floor,
    Walls,
}
impl crate::App {
    pub fn assets_window(&mut self, ctx: &egui::Context) {
        if !self.ui.assets.open {
            return;
        }
        for (asset, result) in self.ui.assets.poll(ctx) {
            match result {
                Ok(path) => self
                    .ui
                    .toast(format!("Downloaded {} → scenes/{path}", asset.name)),
                // The user's own Cancel is not a failure (`assets::save_stream` reports it so).
                Err(e) if e == "Cancelled" => self.ui.toast(format!("Cancelled {}", asset.name)),
                // Failures stay in the strip under the details, where the download was started.
                Err(e) => self.ui.assets.failed.push((asset.name, e)),
            }
        }
        let mut open = true;
        let mut action = None;
        let selection = self.ui.selection;
        // The same frame as the scene browser, over the viewport: sources and kinds at the left,
        // the cards, and the selected asset's details and actions at the right.
        let rect = crate::ui::central_rect(ctx);
        crate::ui::skin::window("Assets")
            .title_bar(false)
            .resizable(false)
            .fixed_rect(rect)
            .frame(crate::ui::skin::panel_frame())
            .show(ctx, |ui| {
                ui.set_min_size(rect.size());
                let b = &mut self.ui.assets;
                let header = |ui: &mut egui::Ui, text: &str| {
                    ui.add_space(10.);
                    crate::ui::skin::heading(ui, text);
                    ui.add_space(2.);
                };
                egui::Panel::left("assets side")
                    .exact_size(200.)
                    .resizable(false)
                    .frame(egui::Frame::new().inner_margin(egui::Margin::symmetric(6, 4)))
                    .show(ui, |ui| {
                        header(ui, "SOURCE");
                        for source in Source::ALL {
                            let on = !b.others && !b.local && b.source == source;
                            if crate::ui::tree_row(ui, 0., on, source.label(), "").clicked() {
                                b.others = false;
                                b.local = false;
                                b.source = source;
                                if !source.kinds().contains(&b.kind) {
                                    b.kind = source.kinds()[0];
                                }
                                b.page = 0;
                            }
                        }
                        if crate::ui::tree_row(ui, 0., b.others, "Other sources…", "").clicked() {
                            b.others = true;
                            b.local = false;
                        }
                        header(ui, "ON THIS COMPUTER");
                        // The count is of everything found, whatever the kind filter says.
                        let count = if b.scan.is_some() && b.installed.is_empty() { "…".to_string() } else { b.installed.len().to_string() };
                        if crate::ui::tree_row(ui, 0., b.local, "Downloaded", &count).on_hover_text("What is already in the scenes folders, from every source. Works offline.").clicked() {
                            b.local = true;
                            b.others = false;
                            b.page = 0;
                        }
                        if b.local {
                            header(ui, "KIND");
                            if crate::ui::tree_row(ui, 0., b.local_kind.is_none(), "All", &count).clicked() {
                                b.local_kind = None;
                                b.page = 0;
                            }
                            for kind in Kind::ALL {
                                let n = b.installed.iter().filter(|(a, _)| a.kind == kind).count();
                                if crate::ui::tree_row(ui, 0., b.local_kind == Some(kind), kind.label(), &n.to_string()).clicked() {
                                    b.local_kind = Some(kind);
                                    b.page = 0;
                                }
                            }
                        } else if !b.others {
                            header(ui, "KIND");
                            for kind in b.source.kinds() {
                                if crate::ui::tree_row(ui, 0., b.kind == *kind, kind.label(), "").clicked() {
                                    b.kind = *kind;
                                    b.page = 0;
                                }
                            }
                        }
                    });
                let (source, kind) = (b.source, b.kind);
                egui::Panel::right("assets details")
                    .exact_size(300.)
                    .resizable(false)
                    .frame(egui::Frame::new().inner_margin(14))
                    .show(ui, |ui| {
                        // Every running download, whatever is selected or listed.
                        if !b.jobs.is_empty() || !b.failed.is_empty() {
                            egui::Panel::bottom("assets downloads")
                                .resizable(false)
                                .frame(egui::Frame::new().inner_margin(egui::Margin { top: 8, ..Default::default() }))
                                .show(ui, |ui| b.download_strip(ui));
                        }
                        if b.others {
                            return;
                        }
                        let listed = |a: &Asset| {
                            if b.local {
                                b.local_kind.is_none_or(|k| a.kind == k)
                            } else {
                                a.source == source && a.kind == kind
                            }
                        };
                        let Some(a) = b.selected.clone().filter(listed) else {
                            note(ui, "Pick an asset. A green dot means it is already in the scenes folder at the chosen resolution.");
                            return;
                        };
                        let width = ui.available_width();
                        match b.thumbs.get(&key(&a)) {
                            Some(t) => {
                                ui.add(egui::Image::new((t.id(), egui::vec2(width, width * 0.6))).maintain_aspect_ratio(true).corner_radius(6.));
                            }
                            None => {
                                placeholder(ui, &a.name, egui::vec2(width, width * 0.6));
                            }
                        }
                        ui.add_space(4.);
                        ui.label(egui::RichText::new(&a.name).heading());
                        note(ui, &a.credit);
                        ui.horizontal_wrapped(|ui| {
                            for t in a.tags.iter().take(12) {
                                ui.label(egui::RichText::new(t).small());
                            }
                        });
                        ui.hyperlink_to(egui::RichText::new(&a.page).small(), &a.page);
                        let mut res = b.res.get(&(a.source, a.kind)).cloned().unwrap_or_else(|| default_res(&a));
                        if !a.resolutions.is_empty() {
                            ui.horizontal(|ui| {
                                ui.label("Resolution");
                                let options: Vec<(usize, &str)> = a.resolutions.iter().map(String::as_str).enumerate().collect();
                                let mut at = a.resolutions.iter().position(|r| *r == res).unwrap_or(usize::MAX);
                                if crate::ui::skin::segment(ui, &mut at, &options) {
                                    res = a.resolutions[at].clone();
                                }
                            });
                            b.res.insert((a.source, a.kind), res.clone());
                        }
                        let path = assets::use_path(&a, &res);
                        let running = b.jobs.iter().find(|j| key(&j.asset) == key(&a) && j.res == res);
                        if let Some(job) = running {
                            // The bar is in the strip at the foot of this pane. Cancel is here
                            // too, because the strip shows only its first rows.
                            note(ui, &format!("Downloading into scenes/{path}…"));
                            let cancelling = job.progress.cancel.load(std::sync::atomic::Ordering::Relaxed);
                            let label = if cancelling { "Cancelling…" } else { "Cancel download" };
                            if ui.add_enabled(!cancelling, crate::ui::skin::Button::new(label)).clicked() {
                                job.progress.cancel.store(true, std::sync::atomic::Ordering::Relaxed);
                            }
                        } else if assets::installed(&a, &res) {
                            note(ui, &format!("In scenes/{path}"));
                            ui.vertical(|ui| match a.kind {
                                Kind::Hdri => {
                                    if crate::ui::skin::primary(ui, "Use as sky map").clicked() {
                                        action = Some(Use::Hdri(path.clone()));
                                    }
                                }
                                Kind::Texture => {
                                    if let Selection::Object(i) = selection
                                        && crate::ui::skin::button(ui, "Apply to the selected object").clicked()
                                    {
                                        action = Some(Use::Texture(path.clone(), Target::Object(i)));
                                    }
                                    if crate::ui::skin::button(ui, "Apply to the floor").clicked() {
                                        action = Some(Use::Texture(path.clone(), Target::Floor));
                                    }
                                    if crate::ui::skin::button(ui, "Apply to the walls").clicked() {
                                        action = Some(Use::Texture(path.clone(), Target::Walls));
                                    }
                                }
                                Kind::Model => {
                                    if crate::ui::skin::primary(ui, "Add to the scene").clicked() {
                                        action = Some(Use::Model(path.clone()));
                                    }
                                }
                            });
                        } else if !b.offline && crate::ui::skin::primary(ui, "Download").on_hover_text(format!("Into scenes/{path}; each file's SHA-256 is pinned in scenes/assets.json")).clicked() {
                            b.failed.clear();
                            b.jobs.push(assets::start(a.clone(), res.clone()));
                        }
                    });
                egui::Frame::new().inner_margin(12).show(ui, |ui| {
                if b.others {
                    ui.horizontal(|ui| {
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if crate::ui::skin::button(ui, "Close").clicked() {
                                open = false;
                            }
                        });
                    });
                    other_sources(ui);
                    return;
                }
                ui.horizontal(|ui| {
                    if crate::ui::skin::text_edit(ui, 220., egui::TextEdit::singleline(&mut b.search).hint_text(field_hint("Search names and tags…")))
                        .changed()
                    {
                        b.page = 0;
                    }
                    if b.local {
                        if b.scan.is_some() {
                            ui.add(egui::Spinner::new());
                        } else if crate::ui::skin::button(ui, "Rescan").on_hover_text("Look through the scenes folders again").clicked() {
                            b.scanned = false;
                        }
                    } else if b.loading.contains_key(&source) {
                        ui.add(egui::Spinner::new());
                        note(ui, "Fetching the catalog…");
                    } else if !b.offline && crate::ui::skin::button(ui, "Refresh").on_hover_text("Fetch the catalog again").clicked() {
                        b.refresh(source);
                    }
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if crate::ui::skin::button(ui, "Close").on_hover_text("Back to the image").clicked() {
                            open = false;
                        }
                    });
                });
                let kind = b.kind;
                let search = b.search.to_lowercase();
                let found = |a: &Asset| {
                    search.is_empty()
                        || a.name.to_lowercase().contains(&search)
                        || a.id.to_lowercase().contains(&search)
                        || a.tags.iter().any(|t| t.contains(&search))
                };
                let local = b.local;
                let list: Vec<Asset> = if local {
                    let only = b.local_kind;
                    b.installed
                        .iter()
                        .map(|(a, _)| a)
                        .filter(|a| only.is_none_or(|k| a.kind == k) && found(a))
                        .cloned()
                        .collect()
                } else {
                    // The categories this catalog really has; a source without them gets no row.
                    let mut offered = [false; HDRI_CATEGORIES.len()];
                    if kind == Kind::Hdri {
                        for a in b.catalog(source).into_iter().flatten().filter(|a| a.kind == kind) {
                            for (on, (tag, _)) in offered.iter_mut().zip(HDRI_CATEGORIES) {
                                *on |= a.tags.iter().any(|t| t == tag);
                            }
                        }
                    }
                    if offered.contains(&true) {
                        let all = std::iter::once((true, ("", "All")));
                        let options: Vec<(&'static str, &str)> = all
                            .chain(offered.into_iter().zip(HDRI_CATEGORIES))
                            .filter(|c| c.0)
                            .map(|(_, option)| option)
                            .collect();
                        let mut chosen = b.category;
                        ui.horizontal(|ui| crate::ui::skin::segment(ui, &mut chosen, &options));
                        if chosen != b.category {
                            b.category = chosen;
                            b.page = 0;
                        }
                    }
                    // A category the list no longer offers (another kind or source) filters nothing.
                    let category = HDRI_CATEGORIES
                        .iter()
                        .zip(offered)
                        .find(|((tag, _), on)| *on && *tag == b.category)
                        .map_or("", |((tag, _), _)| *tag);
                    b.catalog(source)
                        .map(|c| {
                            c.iter()
                                .filter(|a| a.kind == kind && found(a))
                                .filter(|a| category.is_empty() || a.tags.iter().any(|t| t == category))
                                .cloned()
                                .collect()
                        })
                        .unwrap_or_default()
                };
                let pages = list.len().div_ceil(PAGE).max(1);
                b.page = b.page.min(pages - 1);
                ui.horizontal(|ui| {
                    let mut line = if local {
                        format!("{} downloaded, in the scenes folders", list.len())
                    } else {
                        format!("{} {}, most downloaded first · {}", list.len(), kind.label().to_lowercase(), source.license())
                    };
                    if !local && !b.status.is_empty() {
                        line += &format!(" · {}", b.status);
                    }
                    note(ui, &line);
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui.add_enabled(b.page + 1 < pages, crate::ui::skin::Button::new("▶")).clicked() {
                            b.page += 1;
                        }
                        ui.label(format!("{} / {pages}", b.page + 1));
                        if ui.add_enabled(b.page > 0, crate::ui::skin::Button::new("◀")).clicked() {
                            b.page -= 1;
                        }
                    });
                });
                egui::ScrollArea::vertical().id_salt("asset grid").auto_shrink([false; 2]).show(ui, |ui| {
                    ui.horizontal_wrapped(|ui| {
                        for a in list.iter().skip(b.page * PAGE).take(PAGE) {
                            b.request_thumbnail(a);
                            let res = b.res.get(&(a.source, a.kind)).cloned().unwrap_or_else(|| default_res(a));
                            let selected = b.selected.as_ref().is_some_and(|s| key(s) == key(a));
                            // The Downloaded list holds only what the scan found, at some resolution.
                            let r = card(ui, a, b.thumbs.get(&key(a)), selected, local || assets::installed(a, &res));
                            if r.clicked() {
                                b.selected = Some(a.clone());
                                // Show the copy that is there (the largest), not a Download button
                                // for the resolution chosen last.
                                let have = b.installed.iter().find(|(i, _)| key(i) == key(a)).map(|(_, have)| have);
                                if local && let Some(have) = have && !have.contains(&res) && let Some(largest) = have.last() {
                                    b.res.insert((a.source, a.kind), largest.clone());
                                }
                            }
                        }
                    });
                    if local {
                        if list.is_empty() && b.scan.is_some() {
                            note(ui, "Looking through the scenes folders…");
                        } else if list.is_empty() && b.installed.is_empty() {
                            note(ui, "Nothing downloaded yet. A source's assets are listed here once its catalog has been fetched (open the source once while online).");
                        } else if list.is_empty() {
                            note(ui, "Nothing here: check the search and the kind.");
                        }
                    } else if list.is_empty() && !b.loading.contains_key(&source) {
                        note(ui, "Nothing here: check the search, or Refresh (needs the internet once; catalogs are cached).");
                    }
                });
                });
            });
        self.ui.assets.open = open;
        if let Some(action) = action {
            self.use_asset(action);
        }
    }
    fn use_asset(&mut self, action: Use) {
        self.ui.checkpoint(&self.settings);
        match action {
            Use::Hdri(path) => {
                self.settings.hdri = path;
                self.ui
                    .toast("HDRI set (Lights > HDRI for strength and rotation)");
            }
            Use::Texture(path, target) => {
                let maps = crate::texture::load(&path);
                let material = match target {
                    Target::Object(i) => self.settings.objects.get_mut(i).map(|o| &mut o.material),
                    Target::Floor => Some(&mut self.settings.floor),
                    Target::Walls => Some(&mut self.settings.walls),
                };
                match (material, maps) {
                    (Some(m), Ok(maps)) => {
                        m.texture = path;
                        crate::texture::apply_averages(m, &maps);
                        self.ui
                            .toast("Texture applied (Texture scale in the material)");
                    }
                    (_, Err(e)) => self.ui.error(format!("{e:#}")),
                    (None, _) => self.ui.error("The selected object is gone"),
                }
            }
            Use::Model(path) => {
                self.ui.import_mesh(crate::mesh::resolve(&path), false);
            }
        }
    }
}
/// Sources without an in-app catalog: what they have, their licence and a link.
fn other_sources(ui: &mut egui::Ui) {
    note(
        ui,
        "These have no public API the app can browse (or need an account). Download in a web browser, then use + Add, Mesh file… (OBJ, glTF, GLB) or Lights > HDRI > Load….",
    );
    egui::ScrollArea::vertical()
        .id_salt("other sources")
        .show(ui, |ui| {
            for (name, about, license, url) in assets::OTHER_SOURCES {
                ui.add_space(4.);
                ui.horizontal(|ui| {
                    ui.label(egui::RichText::new(name).strong());
                    ui.label(egui::RichText::new(license).small().color(ACCENT));
                });
                ui.label(about);
                ui.hyperlink_to(egui::RichText::new(url).small(), url);
            }
        });
}
