//! The Assets window (roadmap item 6): browse Poly Haven, ambientCG and the Khronos glTF
//! samples, download at a chosen resolution, and use the result in one click (HDRI, texture
//! set on the selected object / floor / walls, or a model added to the scene). Catalogs,
//! downloads and thumbnails all run on background threads (`assets.rs`).
use crate::assets::{self, Asset, Job, Kind, Source};
use crate::ui::{ACCENT, Selection, note, placeholder};
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::mpsc::{Receiver, Sender, channel};
use std::time::Duration;

const PAGE: usize = 48;
type CatalogResult = Result<Vec<Asset>, String>;

pub struct Browser {
    pub open: bool,
    /// Catalog tabs, or the list of other sources.
    others: bool,
    source: Source,
    kind: Kind,
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
    /// Self-test: a catalog to show without the network.
    pub offline: bool,
}
impl Default for Browser {
    fn default() -> Self {
        Self {
            open: false,
            others: false,
            source: Source::PolyHaven,
            kind: Kind::Hdri,
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
        if busy || !self.jobs.is_empty() {
            ctx.request_repaint_after(Duration::from_millis(150));
        }
        finished
    }
}
/// A thumbnail card: image (or initial), name, and a green dot once downloaded.
fn card(
    ui: &mut egui::Ui,
    a: &Asset,
    texture: Option<&egui::TextureHandle>,
    selected: bool,
    installed: bool,
) -> egui::Response {
    let size = egui::vec2(128., 128.);
    let (rect, r) = ui.allocate_exact_size(egui::vec2(size.x, size.y + 18.), egui::Sense::click());
    let image = egui::Rect::from_min_size(rect.min, size);
    match texture {
        Some(t) => {
            // Fit inside the square, keeping the aspect ratio (Khronos screenshots are wide).
            let [w, h] = t.size().map(|v| v as f32);
            let scale = (size.x / w).min(size.y / h);
            let fit = egui::Rect::from_center_size(image.center(), egui::vec2(w, h) * scale);
            ui.painter()
                .rect_filled(image, 3., egui::Color32::from_gray(24));
            egui::Image::new((t.id(), fit.size()))
                .corner_radius(3.)
                .paint_at(ui, fit);
        }
        None => {
            ui.painter()
                .rect_filled(image, 3., egui::Color32::from_gray(38));
            ui.painter().text(
                image.center(),
                egui::Align2::CENTER_CENTER,
                a.name.chars().next().unwrap_or('?'),
                egui::FontId::proportional(40.),
                egui::Color32::from_gray(110),
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
    if selected || r.hovered() {
        let color = if selected {
            ACCENT
        } else {
            egui::Color32::from_gray(150)
        };
        ui.painter().rect_stroke(
            image.expand(1.),
            4.,
            egui::Stroke::new(2_f32, color),
            egui::StrokeKind::Outside,
        );
    }
    let name: String = if a.name.chars().count() > 20 {
        a.name.chars().take(19).collect::<String>() + "…"
    } else {
        a.name.clone()
    };
    ui.painter().text(
        egui::pos2(rect.min.x + 2., image.max.y + 3.),
        egui::Align2::LEFT_TOP,
        name,
        egui::FontId::proportional(12.),
        if selected {
            ACCENT
        } else {
            ui.visuals().text_color()
        },
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
                Err(e) => self.ui.error(format!("{}: {e}", asset.name)),
            }
        }
        let mut open = true;
        let mut action = None;
        let selection = self.ui.selection;
        egui::Window::new("Assets")
            .open(&mut open)
            .default_size([820., 640.])
            .default_pos([140., 60.])
            .collapsible(false)
            .show(ctx, |ui| {
                let b = &mut self.ui.assets;
                ui.horizontal(|ui| {
                    for source in Source::ALL {
                        if ui.selectable_label(!b.others && b.source == source, source.label()).clicked() {
                            b.others = false;
                            b.source = source;
                            if !source.kinds().contains(&b.kind) {
                                b.kind = source.kinds()[0];
                            }
                            b.page = 0;
                        }
                    }
                    if ui.selectable_label(b.others, "Other sources").clicked() {
                        b.others = true;
                    }
                });
                ui.separator();
                if b.others {
                    other_sources(ui);
                    return;
                }
                let source = b.source;
                ui.horizontal(|ui| {
                    if source.kinds().len() > 1 {
                        for kind in source.kinds() {
                            if ui.selectable_label(b.kind == *kind, kind.label()).clicked() {
                                b.kind = *kind;
                                b.page = 0;
                            }
                        }
                        ui.separator();
                    }
                    if ui
                        .add(egui::TextEdit::singleline(&mut b.search).hint_text("Search names and tags…").desired_width(220.))
                        .changed()
                    {
                        b.page = 0;
                    }
                    if b.loading.contains_key(&source) {
                        ui.add(egui::Spinner::new());
                        note(ui, "Fetching the catalog…");
                    } else if !b.offline && ui.button("Refresh").on_hover_text("Fetch the catalog again").clicked() {
                        b.refresh(source);
                    }
                });
                let kind = b.kind;
                let search = b.search.to_lowercase();
                let list: Vec<Asset> = b
                    .catalog(source)
                    .map(|c| {
                        c.iter()
                            .filter(|a| a.kind == kind)
                            .filter(|a| {
                                search.is_empty()
                                    || a.name.to_lowercase().contains(&search)
                                    || a.id.to_lowercase().contains(&search)
                                    || a.tags.iter().any(|t| t.contains(&search))
                            })
                            .cloned()
                            .collect()
                    })
                    .unwrap_or_default();
                let pages = list.len().div_ceil(PAGE).max(1);
                b.page = b.page.min(pages - 1);
                ui.horizontal(|ui| {
                    let mut line = format!("{} {}, most downloaded first · {}", list.len(), kind.label().to_lowercase(), source.license());
                    if !b.status.is_empty() {
                        line += &format!(" · {}", b.status);
                    }
                    note(ui, &line);
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui.add_enabled(b.page + 1 < pages, egui::Button::new("▶")).clicked() {
                            b.page += 1;
                        }
                        ui.label(format!("{} / {pages}", b.page + 1));
                        if ui.add_enabled(b.page > 0, egui::Button::new("◀")).clicked() {
                            b.page -= 1;
                        }
                    });
                });
                egui::ScrollArea::vertical().id_salt("asset grid").max_height(420.).auto_shrink([false, true]).show(ui, |ui| {
                    ui.horizontal_wrapped(|ui| {
                        for a in list.iter().skip(b.page * PAGE).take(PAGE) {
                            b.request_thumbnail(a);
                            let res = b.res.get(&(a.source, a.kind)).cloned().unwrap_or_else(|| default_res(a));
                            let selected = b.selected.as_ref().is_some_and(|s| key(s) == key(a));
                            let r = card(ui, a, b.thumbs.get(&key(a)), selected, assets::installed(a, &res));
                            if r.clicked() {
                                b.selected = Some(a.clone());
                            }
                        }
                    });
                    if list.is_empty() && !b.loading.contains_key(&source) {
                        note(ui, "Nothing here: check the search, or Refresh (needs the internet once; catalogs are cached).");
                    }
                });
                ui.separator();
                let Some(a) = b.selected.clone().filter(|a| a.source == source && a.kind == kind) else {
                    note(ui, "Pick an asset. A green dot means it is already in the scenes folder at the chosen resolution.");
                    return;
                };
                ui.horizontal(|ui| {
                    match b.thumbs.get(&key(&a)) {
                        Some(t) => {
                            ui.add(egui::Image::new((t.id(), egui::vec2(160., 160.))).maintain_aspect_ratio(true).corner_radius(4.));
                        }
                        None => {
                            placeholder(ui, &a.name, egui::vec2(160., 160.));
                        }
                    }
                    ui.vertical(|ui| {
                        ui.label(egui::RichText::new(&a.name).heading());
                        note(ui, &a.credit);
                        ui.horizontal_wrapped(|ui| {
                            for t in a.tags.iter().take(12) {
                                ui.label(egui::RichText::new(t).small().weak());
                            }
                        });
                        ui.hyperlink_to(egui::RichText::new(&a.page).small(), &a.page);
                        let mut res = b.res.get(&(a.source, a.kind)).cloned().unwrap_or_else(|| default_res(&a));
                        if !a.resolutions.is_empty() {
                            ui.horizontal(|ui| {
                                ui.label("Resolution");
                                for r in &a.resolutions {
                                    ui.selectable_value(&mut res, r.clone(), r);
                                }
                            });
                            b.res.insert((a.source, a.kind), res.clone());
                        }
                        let path = assets::use_path(&a, &res);
                        let running = b.jobs.iter().find(|j| key(&j.asset) == key(&a) && j.res == res);
                        if let Some(job) = running {
                            let done = job.progress.done.load(std::sync::atomic::Ordering::Relaxed);
                            let total = job.progress.total.load(std::sync::atomic::Ordering::Relaxed);
                            ui.horizontal(|ui| {
                                let fraction = if total > 0 { done as f32 / total as f32 } else { 0. };
                                ui.add(egui::ProgressBar::new(fraction).desired_width(220.).text(format!("{:.1} MB", done as f64 / 1e6)));
                                if ui.button("Cancel").clicked() {
                                    job.progress.cancel.store(true, std::sync::atomic::Ordering::Relaxed);
                                }
                            });
                        } else if assets::installed(&a, &res) {
                            note(ui, &format!("In scenes/{path}"));
                            ui.horizontal(|ui| match a.kind {
                                Kind::Hdri => {
                                    if ui.button("Use as HDRI").clicked() {
                                        action = Some(Use::Hdri(path.clone()));
                                    }
                                }
                                Kind::Texture => {
                                    if let Selection::Object(i) = selection
                                        && ui.button("Apply to the selected object").clicked()
                                    {
                                        action = Some(Use::Texture(path.clone(), Target::Object(i)));
                                    }
                                    if ui.button("Apply to the floor").clicked() {
                                        action = Some(Use::Texture(path.clone(), Target::Floor));
                                    }
                                    if ui.button("Apply to the walls").clicked() {
                                        action = Some(Use::Texture(path.clone(), Target::Walls));
                                    }
                                }
                                Kind::Model => {
                                    if ui.button("Add to the scene").clicked() {
                                        action = Some(Use::Model(path.clone()));
                                    }
                                }
                            });
                        } else if !b.offline && ui.button("Download").on_hover_text(format!("Into scenes/{path}; each file's SHA-256 is pinned in scenes/assets.json")).clicked() {
                            b.jobs.push(assets::start(a.clone(), res.clone()));
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
                    .toast("HDRI set (Stage → HDRI for strength and rotation)");
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
                let full = crate::mesh::resolve(&path);
                match crate::mesh::import_objects(&full) {
                    Ok(objects) => {
                        let room = crate::scene::MAX_OBJECTS - self.settings.objects.len();
                        let count = objects.len().min(room);
                        self.settings
                            .objects
                            .extend(objects.into_iter().take(count));
                        if count > 0 {
                            self.ui.selection =
                                Selection::Object(self.settings.objects.len() - count);
                        }
                        self.ui.toast(format!("Added {count} object(s)"));
                    }
                    Err(e) => self.ui.error(format!("{e:#}")),
                }
            }
        }
    }
}
/// Sources without an in-app catalog: what they have, their licence and a link.
fn other_sources(ui: &mut egui::Ui) {
    note(
        ui,
        "These have no public API the app can browse (or need an account). Download in a web browser, then use + Mesh… (OBJ, glTF, GLB) or Stage → HDRI → Load….",
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
