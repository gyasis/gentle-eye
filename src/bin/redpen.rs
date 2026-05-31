//! redpen — native visual annotator for gentle-eye (the human→agent visual-context tool).
//!
//! Built ONLY with `--features ui` (keeps egui/wgpu out of the default
//! `gentle-eye` MCP/CLI build). PRD: `gentle_eye_redpen_native_annotator_2026-05-31`.
//!
//! MVP: capture-on-launch (or `--input PATH`) → mouse-draw box(es); each box is a
//! valid gentle-eye `target` (pixel→normalized via `geometry::pixel_to_norm`,
//! saved to `TargetStore`) → save a flattened PNG + a NormRect sidecar JSON to
//! `~/.gentle-eye/redpen/`. Enter = save+quit, Esc = cancel.

use eframe::egui::{self, Color32, Pos2, Rect, Sense, Stroke};
use gentle_eye::target::crop::crop_bgra;
use gentle_eye::target::geometry::pixel_to_norm;
use gentle_eye::target::model::{PixelRect, Target, TargetSource};
use gentle_eye::target::store::TargetStore;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

fn main() -> eframe::Result {
    // `redpen --list` / `--displays`: print the monitor catalogue and exit (no GUI).
    // Use it to find which index is your real screen on a multi-monitor setup.
    let argv: Vec<String> = std::env::args().collect();
    if argv.iter().any(|a| a == "--list" || a == "--displays") {
        match gentle_eye::capture::display::DisplayManager::list_available() {
            Ok(ds) => {
                println!("redpen — pick a display index with `--display IDX`:");
                for d in &ds {
                    println!("  {}", d.auto_name);
                }
            }
            Err(e) => eprintln!("redpen: list displays: {e}"),
        }
        return Ok(());
    }

    let (rgba, w, h) = match load_image() {
        Ok(v) => v,
        Err(e) => {
            eprintln!("redpen: {e}");
            std::process::exit(1);
        }
    };
    eframe::run_native(
        "redpen — gentle-eye annotator",
        eframe::NativeOptions::default(),
        Box::new(move |_cc| Ok(Box::new(RedpenApp::new(rgba, w, h)))),
    )
}

/// A drawn box (image-pixel coords) that becomes a named target.
struct NamedBox {
    name: String,
    rect: Rect, // in IMAGE-pixel space
}

struct RedpenApp {
    rgba: Vec<u8>,
    iw: usize,
    ih: usize,
    texture: Option<egui::TextureHandle>,
    boxes: Vec<NamedBox>,
    drag_start: Option<Pos2>, // image-pixel space
    status: String,
}

impl RedpenApp {
    fn new(rgba: Vec<u8>, iw: usize, ih: usize) -> Self {
        Self {
            rgba,
            iw,
            ih,
            texture: None,
            boxes: Vec::new(),
            drag_start: None,
            status: format!("{iw}×{ih} — drag to draw a box, name it, then Save (Enter). Esc cancels."),
        }
    }

    /// Persist each box as a gentle-eye target + write the flattened PNG + sidecar JSON.
    fn save(&mut self) {
        if self.boxes.is_empty() {
            self.status = "nothing to save (draw a box first)".into();
            return;
        }
        // 1. boxes → targets (pixel → normalized 0–1 via the shipped geometry).
        let mut store = match TargetStore::load() {
            Ok(s) => s,
            Err(e) => {
                self.status = format!("target store: {e}");
                return;
            }
        };
        for b in &self.boxes {
            let px = PixelRect {
                x: b.rect.min.x.round().max(0.0) as u32,
                y: b.rect.min.y.round().max(0.0) as u32,
                w: b.rect.width().round().max(1.0) as u32,
                h: b.rect.height().round().max(1.0) as u32,
            };
            let region = pixel_to_norm(px, (self.iw as u32, self.ih as u32), (0, 0));
            let mut t = Target::new(b.name.clone(), TargetSource::Display { index: 0 }, region);
            t.active = true; // last one drawn stays active (store enforces one)
            store.add(t);
        }
        if let Err(e) = store.save() {
            self.status = format!("save targets: {e}");
            return;
        }

        // 2. flattened PNG (image + red boxes) + sidecar JSON → ~/.gentle-eye/redpen/<ts>.
        let dir = std::path::Path::new(&std::env::var("HOME").unwrap_or_default())
            .join(".gentle-eye/redpen");
        let _ = std::fs::create_dir_all(&dir);
        let ts = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        let png_path = dir.join(format!("{ts}.png"));
        let json_path = dir.join(format!("{ts}.json"));

        if let Some(mut img) =
            image::RgbaImage::from_raw(self.iw as u32, self.ih as u32, self.rgba.clone())
        {
            let red = image::Rgba([224u8, 36, 27, 255]);
            for b in &self.boxes {
                // 3px hollow rect (3 nested) for visibility.
                for off in 0..3i32 {
                    let r = imageproc::rect::Rect::at(
                        b.rect.min.x as i32 + off,
                        b.rect.min.y as i32 + off,
                    )
                    .of_size(
                        (b.rect.width() as i32 - 2 * off).max(1) as u32,
                        (b.rect.height() as i32 - 2 * off).max(1) as u32,
                    );
                    imageproc::drawing::draw_hollow_rect_mut(&mut img, r, red);
                }
            }
            let _ = img.save(&png_path);
        }

        // sidecar: labeled normalized rects — the spatial-reasoning payload for the LLM.
        let entries: Vec<_> = self
            .boxes
            .iter()
            .map(|b| {
                let px = PixelRect {
                    x: b.rect.min.x.round().max(0.0) as u32,
                    y: b.rect.min.y.round().max(0.0) as u32,
                    w: b.rect.width().round().max(1.0) as u32,
                    h: b.rect.height().round().max(1.0) as u32,
                };
                let n = pixel_to_norm(px, (self.iw as u32, self.ih as u32), (0, 0));
                serde_json::json!({ "label": b.name, "rect": [n.x, n.y, n.w, n.h] })
            })
            .collect();
        let _ = std::fs::write(
            &json_path,
            serde_json::to_string_pretty(&serde_json::json!({
                "image": png_path.to_string_lossy(),
                "size": [self.iw, self.ih],
                "targets": entries,
            }))
            .unwrap_or_default(),
        );

        // Files are flushed; exit the process to close the window reliably.
        // (ViewportCommand::Close was not honored on the root viewport here.)
        eprintln!("saved {} target(s) → {}", self.boxes.len(), png_path.display());
        std::process::exit(0);
    }
}

impl eframe::App for RedpenApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        // Texture (once).
        if self.texture.is_none() {
            let ci = egui::ColorImage::from_rgba_unmultiplied([self.iw, self.ih], &self.rgba);
            self.texture = Some(ctx.load_texture("shot", ci, egui::TextureOptions::default()));
        }

        // Esc = cancel: quit immediately, nothing saved.
        if ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
            std::process::exit(0);
        }
        // Enter = save+quit. Captured here (canvas focused) AND per name-field
        // below — a focused text edit consumes Enter, so we check both so it
        // works regardless of where focus is.
        let mut save_now = ctx.input(|i| i.key_pressed(egui::Key::Enter));

        egui::TopBottomPanel::bottom("bar").show(ctx, |ui| {
            ui.horizontal_wrapped(|ui| {
                if ui.button("Save & Quit (Enter)").clicked() {
                    save_now = true;
                }
                if ui.button("Undo last box").clicked() {
                    self.boxes.pop();
                }
                if ui.button("Cancel (Esc)").clicked() {
                    std::process::exit(0);
                }
                ui.separator();
                ui.label(&self.status);
            });
            for b in &mut self.boxes {
                ui.horizontal(|ui| {
                    ui.label("target:");
                    let r = ui.text_edit_singleline(&mut b.name);
                    if r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                        save_now = true;
                    }
                });
            }
        });

        egui::CentralPanel::default().show(ctx, |ui| {
            let (resp, painter) = ui.allocate_painter(ui.available_size(), Sense::click_and_drag());
            let panel = resp.rect;
            // Fit the image into the panel, centered.
            let (iw, ih) = (self.iw as f32, self.ih as f32);
            let scale = (panel.width() / iw).min(panel.height() / ih).max(0.0001);
            let draw = egui::vec2(iw * scale, ih * scale);
            let origin = panel.min + (panel.size() - draw) / 2.0;
            let img_rect = Rect::from_min_size(origin, draw);
            if let Some(tex) = &self.texture {
                painter.image(
                    tex.id(),
                    img_rect,
                    Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
                    Color32::WHITE,
                );
            }
            let to_img = |p: Pos2| -> Pos2 { egui::pos2((p.x - origin.x) / scale, (p.y - origin.y) / scale) };
            let to_screen = |p: Pos2| -> Pos2 { egui::pos2(origin.x + p.x * scale, origin.y + p.y * scale) };

            // Drawing interaction (image-pixel space).
            if resp.drag_started() {
                self.drag_start = resp.interact_pointer_pos().map(to_img);
            }
            let mut draft: Option<Rect> = None;
            if let (Some(start), Some(cur)) = (self.drag_start, resp.interact_pointer_pos().map(to_img)) {
                draft = Some(Rect::from_two_pos(start, cur));
            }
            if resp.drag_stopped() {
                if let (Some(start), Some(end)) = (self.drag_start, resp.interact_pointer_pos().map(to_img)) {
                    let r = Rect::from_two_pos(start, end);
                    if r.width() > 3.0 && r.height() > 3.0 {
                        let n = self.boxes.len() + 1;
                        self.boxes.push(NamedBox { name: format!("box_{n}"), rect: r });
                    }
                }
                self.drag_start = None;
            }

            // Draw existing boxes + the live draft (map image-px → screen).
            let red = Stroke::new(2.0, Color32::from_rgb(224, 36, 27));
            for b in &self.boxes {
                let sr = Rect::from_two_pos(to_screen(b.rect.min), to_screen(b.rect.max));
                painter.rect_stroke(sr, egui::Rounding::ZERO, red);
                painter.text(sr.min, egui::Align2::LEFT_BOTTOM, &b.name, egui::FontId::proportional(12.0), red.color);
            }
            if let Some(r) = draft {
                let sr = Rect::from_two_pos(to_screen(r.min), to_screen(r.max));
                painter.rect_stroke(sr, egui::Rounding::ZERO, red);
            }
        });

        // Save after the panels so we don't alias `self` inside the closures.
        // save() writes the artifacts then exits the process (= closes window).
        if save_now {
            self.save();
        }
    }
}

/// Capture-on-launch unless `--input PATH` is given (load that image).
///
/// `--display IDX` selects which monitor to grab (default 0). On a multi-monitor
/// setup a blank/unused screen can be index 0 — use `redpen --list` to see the
/// catalogue and pick the index whose geometry matches your working screen.
fn load_image() -> Result<(Vec<u8>, usize, usize), String> {
    let args: Vec<String> = std::env::args().collect();
    if let Some(i) = args.iter().position(|a| a == "--input") {
        let path = args.get(i + 1).ok_or("--input needs a PATH")?;
        let img = image::open(path).map_err(|e| format!("open {path}: {e}"))?.to_rgba8();
        let (w, h) = (img.width() as usize, img.height() as usize);
        return Ok((img.into_raw(), w, h));
    }
    use gentle_eye::capture::screen::ScreenCapturer;
    // --display IDX (default 0). Mirrors `gentle-eye screenshot --display`.
    let display: usize = args
        .iter()
        .position(|a| a == "--display")
        .and_then(|i| args.get(i + 1))
        .and_then(|s| s.parse().ok())
        .unwrap_or(0);
    let mut cap =
        ScreenCapturer::new(display).map_err(|e| format!("screen capture unavailable: {e}"))?;
    let (w, h) = (cap.width(), cap.height());
    let buf = cap
        .capture_frame(Duration::from_secs(2))
        .map_err(|e| format!("capture failed: {e}"))?;
    let stride = buf.len().checked_div(h).unwrap_or(w * 4);
    let rect = PixelRect { x: 0, y: 0, w: w as u32, h: h as u32 };
    let (bgra, _, _) = crop_bgra(&buf, w, h, stride, rect).map_err(|e| format!("repack: {e}"))?;
    // BGRA → RGBA, forcing alpha opaque: X11 root-window capture leaves alpha=0,
    // which egui's `from_rgba_unmultiplied` would render as fully transparent (a
    // black canvas). A screenshot has no transparency, so pin A=255.
    let mut rgba = vec![0u8; w * h * 4];
    for i in 0..(w * h) {
        rgba[i * 4] = bgra[i * 4 + 2]; // R <- B-position byte (BGRA → RGBA)
        rgba[i * 4 + 1] = bgra[i * 4 + 1];
        rgba[i * 4 + 2] = bgra[i * 4];
        rgba[i * 4 + 3] = 255;
    }
    Ok((rgba, w, h))
}
