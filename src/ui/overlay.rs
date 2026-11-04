//! The viewport overlay: gizmos, outlines, A/B split handle, and transforms.
use super::*;

impl App {
    /// Gizmo, light outlines and the transform header, drawn under the panels.
    pub(super) fn overlay(&mut self, ctx: &egui::Context) {
        let g = self.gpu.as_ref().unwrap();
        if self.ui.meter
            && let (Some(crate::gpu::Probe { linear, albedo, .. }), Some(p)) =
                (g.probe_result, ctx.pointer_hover_pos())
        {
            // 1.0 in scene light is 80 nits, as in the HDR output. Illuminance assumes a
            // Lambertian surface: E = π L / albedo.
            let l = 0.2126 * linear[0] + 0.7152 * linear[1] + 0.0722 * linear[2];
            let a = 0.2126 * albedo[0] + 0.7152 * albedo[1] + 0.0722 * albedo[2];
            let nits = l * 80.;
            let text = if a > 0.02 {
                format!(
                    "{nits:.1} nits · {:.0} lux",
                    std::f32::consts::PI * nits / a
                )
            } else {
                format!("{nits:.1} nits")
            };
            ctx.debug_painter().text(
                p + egui::vec2(16., 16.),
                egui::Align2::LEFT_TOP,
                text,
                egui::FontId::monospace(13.),
                egui::Color32::from_rgb(255, 230, 140),
            );
        }
        let view = gizmo::View::new(&self.settings, [g.width, g.height]);
        let ppp = ctx.pixels_per_point();
        let painter = ctx.layer_painter(egui::LayerId::new(
            egui::Order::Background,
            egui::Id::new("gizmo"),
        ));
        let targets = self.ui.targets();
        gizmo::draw_lights(&painter, ppp, &view, &self.settings, &targets);
        // With "Beam in image" the rods show the beam in the render itself; the overlay line on
        // top of them would hide what they light.
        if self.settings.puzzle.enabled && !self.settings.puzzle.beam && self.settings.perspective()
        {
            self.laser.draw(&painter, ppp, &view);
        }
        // The camera path (timeline *Camera path*), while it isn't the camera's own move.
        if self.settings.animation.camera_path
            && !self.timeline.playing
            && self.settings.perspective()
        {
            let points = self
                .settings
                .animation
                .camera_path_points(&self.settings, 24);
            for w in points.windows(2) {
                if let Some((a, b)) = view.segment(w[0], w[1]) {
                    painter.line_segment(
                        [
                            egui::pos2(a.x / ppp, a.y / ppp),
                            egui::pos2(b.x / ppp, b.y / ppp),
                        ],
                        egui::Stroke::new(2_f32, egui::Color32::from_rgb(255, 200, 80)),
                    );
                }
            }
        }
        if self.ui.paths.open && self.settings.perspective() {
            crate::paths::draw(
                &painter,
                ppp,
                &view,
                self.settings.distance.max(1.),
                &self.ui.paths,
            );
        }
        // A/B compare: the split line can be dragged (a 12-point strip over the line).
        if self.settings.compare {
            let screen = ctx.screen_rect();
            let x = screen.left() + self.settings.split * screen.width();
            egui::Area::new(egui::Id::new("ab split"))
                .fixed_pos(egui::pos2(x - 6., screen.top()))
                .order(egui::Order::Background)
                .show(ctx, |ui| {
                    let (rect, r) = ui
                        .allocate_exact_size(egui::vec2(12., screen.height()), egui::Sense::drag());
                    let r = r
                        .on_hover_cursor(egui::CursorIcon::ResizeHorizontal)
                        .on_hover_text("Drag to move the A/B split");
                    if r.dragged() {
                        self.settings.split =
                            (self.settings.split + r.drag_delta().x / screen.width()).clamp(0., 1.);
                    }
                    let grip = egui::pos2(rect.center().x, rect.center().y);
                    let fill = if r.hovered() || r.dragged() {
                        ACCENT
                    } else {
                        egui::Color32::from_white_alpha(160)
                    };
                    ui.painter().circle_filled(grip, 6., fill);
                });
        }
        // Battle mode (track 15): the quadrant split and each integrator's samples and GPU time.
        if self.settings.battle {
            let screen = ctx.screen_rect();
            let painter = ctx.layer_painter(egui::LayerId::new(
                egui::Order::Background,
                egui::Id::new("battle"),
            ));
            let c = screen.center();
            let line = egui::Stroke::new(1.5_f32, egui::Color32::from_white_alpha(150));
            painter.line_segment(
                [
                    egui::pos2(c.x, screen.top()),
                    egui::pos2(c.x, screen.bottom()),
                ],
                line,
            );
            painter.line_segment(
                [
                    egui::pos2(screen.left(), c.y),
                    egui::pos2(screen.right(), c.y),
                ],
                line,
            );
            let g = self.gpu.as_ref().unwrap();
            for (i, q) in g.battle.iter().enumerate() {
                let at = egui::pos2(
                    if i % 2 == 0 {
                        screen.left() + 12.
                    } else {
                        c.x + 12.
                    },
                    // Below the two toolbar rows.
                    if i < 2 { screen.top() + 62. } else { c.y + 10. },
                );
                let text = format!(
                    "{}\n{} spp · {:.1} s GPU",
                    crate::gpu::BATTLE_NAMES[i],
                    q.samples,
                    q.ms / 1000.
                );
                let font = egui::FontId::proportional(14.);
                painter.text(
                    at + egui::vec2(1., 1.),
                    egui::Align2::LEFT_TOP,
                    &text,
                    font.clone(),
                    egui::Color32::BLACK,
                );
                painter.text(at, egui::Align2::LEFT_TOP, text, font, BATTLE_COLORS[i]);
            }
        }
        // Selection and hover outlines: silhouettes from the GPU's object ids, or the projected
        // box for a hidden object (it has no pixels). Not while a video export renders at its
        // own size (the ids don't match the window then).
        if self.settings.perspective() && g.swap_size == [g.width, g.height] {
            let outline = |i: usize, width: f32, color: egui::Color32| {
                let Some(o) = self.settings.objects.get(i) else {
                    return;
                };
                let corners = crate::geometry::object_corners(o);
                if o.visible {
                    gizmo::draw_silhouette(&painter, ppp, &view, &corners, i, width * ppp, color);
                } else {
                    let stroke = egui::Stroke::new(width.min(1.5), color);
                    gizmo::draw_box_outline(&painter, ppp, &view, &corners, stroke);
                }
            };
            if let Some(i) = self.ui.hovered
                && !self.ui.is_selected(Selection::Object(i))
                && self.ui.transform.is_none()
            {
                outline(i, 1., egui::Color32::from_white_alpha(110));
            }
            for t in &targets {
                if let Selection::Object(i) = *t {
                    // The active item brighter than the rest of the selection.
                    if *t == self.ui.selection {
                        outline(i, 2., ACCENT);
                    } else {
                        outline(i, 1.5, ACCENT.gamma_multiply(0.6));
                    }
                }
            }
            // The 3D cursor, while it is the pivot: a red and white ring with a cross.
            if self.ui.pivot == gizmo::Pivot::Cursor
                && let Some(c) = view.project(self.ui.cursor3d)
            {
                let c = egui::pos2(c.x / ppp, c.y / ppp);
                for k in 0..8 {
                    let (a0, a1) = (
                        k as f32 / 8. * std::f32::consts::TAU,
                        (k + 1) as f32 / 8. * std::f32::consts::TAU,
                    );
                    let color = if k % 2 == 0 {
                        egui::Color32::from_rgb(232, 72, 82)
                    } else {
                        egui::Color32::WHITE
                    };
                    painter.line_segment(
                        [
                            c + 9. * egui::vec2(a0.cos(), a0.sin()),
                            c + 9. * egui::vec2(a1.cos(), a1.sin()),
                        ],
                        egui::Stroke::new(1.5_f32, color),
                    );
                }
                let cross = egui::Stroke::new(1_f32, egui::Color32::from_black_alpha(200));
                painter.line_segment([c - egui::vec2(14., 0.), c + egui::vec2(14., 0.)], cross);
                painter.line_segment([c - egui::vec2(0., 14.), c + egui::vec2(0., 14.)], cross);
            }
        }
        // Box select in progress: the rectangle from where the drag started to the pointer.
        if let Some(a) = self.ui.box_start {
            let b = egui::pos2(self.cursor[0] as f32 / ppp, self.cursor[1] as f32 / ppp);
            let r = egui::Rect::from_two_pos(a, b);
            painter.rect_filled(
                r,
                0.,
                egui::Color32::from_rgba_unmultiplied(160, 200, 255, 22),
            );
            painter.rect_stroke(
                r,
                0.,
                egui::Stroke::new(1_f32, egui::Color32::from_white_alpha(170)),
                egui::StrokeKind::Inside,
            );
        }
        self.ui.hover = None;
        self.ui.hover_inner = false;
        if let Some(t) = &self.ui.transform {
            if let Some(f) = t.frame() {
                gizmo::draw(
                    &painter,
                    ppp,
                    &view,
                    &f,
                    t.local,
                    self.ui.tool,
                    None,
                    Some(t),
                );
            }
            egui::Area::new(egui::Id::new("transform header"))
                .anchor(egui::Align2::CENTER_TOP, [0., 44.])
                .order(egui::Order::Foreground)
                .interactable(false)
                .show(ctx, |ui| {
                    egui::Frame::popup(ui.style()).show(ui, |ui| {
                        ui.label(egui::RichText::new(&self.ui.header).strong());
                        note(ui, "X / Y / Z axis (again: local, again: free) · Shift+X/Y/Z plane · type a value (a free move: Tab for the next axis) · Ctrl snap · Shift fine · click or Enter to confirm · right-click or Esc to cancel");
                    });
                });
            return;
        }
        let mode = self.tool_mode();
        // The gizmo's projection math assumes a perspective camera.
        if self.ui.tool == gizmo::Tool::Select
            || !gizmo::allows(&targets, mode)
            || !self.settings.perspective()
        {
            return;
        }
        let Some(f) = gizmo::group_frame(&self.settings, &targets, self.ui.pivot, self.ui.cursor3d)
        else {
            return;
        };
        if !ctx.is_pointer_over_area()
            && let Some(p) = ctx.pointer_hover_pos()
        {
            let mouse = glam::Vec2::new(p.x * ppp, p.y * ppp);
            self.ui.hover = gizmo::hit(&view, &f, self.ui.local, self.ui.tool, mouse);
            self.ui.hover_inner = gizmo::inner(&view, &f, self.ui.local, self.ui.tool, mouse);
        }
        gizmo::draw(
            &painter,
            ppp,
            &view,
            &f,
            self.ui.local,
            self.ui.tool,
            self.ui.hover,
            None,
        );
    }
    /// Starts a transform of the selection: from the handle under the pointer, or modal.
    pub fn start_transform(&mut self, mode: gizmo::Mode, handle: Option<gizmo::Handle>) -> bool {
        let targets = self.ui.targets();
        if targets.iter().any(|t| {
            matches!(t, Selection::Object(i) if self.settings.objects.get(*i).is_some_and(|o| o.locked))
        }) {
            self.ui.toast("Locked: this object can't be moved");
            return false;
        }
        if targets.is_empty()
            || !gizmo::allows(&targets, mode)
            || gizmo::frame(&self.settings, targets[0]).is_none()
        {
            return false;
        }
        let mouse = glam::Vec2::new(self.cursor[0] as f32, self.cursor[1] as f32);
        let mut t = match handle {
            Some(h) => gizmo::Transform::from_handle(
                mode,
                h,
                targets,
                mouse,
                &self.settings,
                self.ui.local,
            ),
            None => gizmo::Transform::new(mode, targets, mouse, &self.settings, self.ui.local),
        };
        t.pivot = self.ui.pivot;
        t.cursor = self.ui.cursor3d;
        t.snap_to = self.ui.snap_to;
        self.ui.transform = Some(t);
        self.update_transform();
        true
    }
    pub fn update_transform(&mut self) {
        let Some(g) = self.gpu.as_ref() else { return };
        let view = gizmo::View::new(&self.settings, [g.width, g.height]);
        if let Some(t) = &mut self.ui.transform {
            t.snap = self.ctrl;
            self.ui.header = t.apply(&mut self.settings, &view);
        }
    }
    pub fn finish_transform(&mut self) {
        if let Some(t) = self.ui.transform.take() {
            // Only the targets count: with physics running the other bodies moved meanwhile.
            let changed = t.targets.iter().any(|sel| match *sel {
                Selection::Object(i) => self.settings.objects.get(i) != t.original.objects.get(i),
                Selection::Light(i) => self.settings.lights.get(i) != t.original.lights.get(i),
                Selection::MainLight => {
                    (
                        self.settings.light_position,
                        self.settings.light_size,
                        self.settings.light_aspect,
                    ) != (
                        t.original.light_position,
                        t.original.light_size,
                        t.original.light_aspect,
                    )
                }
                _ => self.settings != t.original,
            });
            if changed {
                self.ui.checkpoint(&t.original);
                // Laser puzzle (track 17): each finished transform of puzzle pieces (objects,
                // not lights or the camera) is one move.
                if self.settings.puzzle.enabled
                    && t.targets.iter().all(|s| matches!(s, Selection::Object(_)))
                {
                    self.laser.moves += 1;
                }
            }
            self.ui.header.clear();
        }
    }
    pub fn cancel_transform(&mut self) {
        if let Some(t) = self.ui.transform.take() {
            t.restore(&mut self.settings);
            if t.undo_on_cancel {
                // The copy made for this move goes too (Blender's Shift+D / Alt-drag).
                self.ui.undo(&mut self.settings);
            }
            self.ui.header.clear();
            self.ui.toast("Transform cancelled");
        }
    }
    /// Keys while a transform runs. Returns true when the key was used.
    pub fn transform_key(&mut self, code: winit::keyboard::KeyCode) -> bool {
        use winit::keyboard::KeyCode as K;
        let Some(t) = &mut self.ui.transform else {
            return false;
        };
        let digit = [
            (K::Digit0, '0'),
            (K::Digit1, '1'),
            (K::Digit2, '2'),
            (K::Digit3, '3'),
            (K::Digit4, '4'),
            (K::Digit5, '5'),
            (K::Digit6, '6'),
            (K::Digit7, '7'),
            (K::Digit8, '8'),
            (K::Digit9, '9'),
            (K::Numpad0, '0'),
            (K::Numpad1, '1'),
            (K::Numpad2, '2'),
            (K::Numpad3, '3'),
            (K::Numpad4, '4'),
            (K::Numpad5, '5'),
            (K::Numpad6, '6'),
            (K::Numpad7, '7'),
            (K::Numpad8, '8'),
            (K::Numpad9, '9'),
            (K::Period, '.'),
            (K::NumpadDecimal, '.'),
            (K::Minus, '-'),
            (K::NumpadSubtract, '-'),
        ]
        .iter()
        .find(|(k, _)| *k == code)
        .map(|(_, c)| *c);
        match code {
            K::KeyX | K::KeyY | K::KeyZ => {
                let axis = match code {
                    K::KeyX => 0,
                    K::KeyY => 1,
                    _ => 2,
                };
                t.constrain(axis, self.shift);
            }
            K::Backspace => {
                t.typed.pop();
            }
            // A free or plane move takes a value per axis: Tab moves to the next one.
            K::Tab
                if t.mode == gizmo::Mode::Move
                    && !matches!(t.constraint, gizmo::Constraint::Axis(_)) =>
            {
                t.next_field();
            }
            K::Enter | K::NumpadEnter => {
                self.finish_transform();
                return true;
            }
            K::Escape => {
                self.cancel_transform();
                return true;
            }
            _ => match digit {
                Some('-') => {
                    // Minus toggles the sign, as in Blender.
                    if t.typed.starts_with('-') {
                        t.typed.remove(0);
                    } else {
                        t.typed.insert(0, '-');
                    }
                }
                Some(c) => t.typed.push(c),
                None => return true,
            },
        }
        self.update_transform();
        true
    }
}
