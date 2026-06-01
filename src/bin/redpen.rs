//! redpen — native visual annotator for gentle-eye (the human→agent visual-context tool).
//!
//! Built ONLY with `--features ui` (keeps egui/wgpu out of the default
//! `gentle-eye` MCP/CLI build). PRD: `gentle_eye_redpen_native_annotator_2026-05-31`.
//!
//! A markup tool, not a crop-picker: freehand **Pen**, **Arrow** (point / "move
//! this here"), and **Box**, in a small color palette. Output is pure visual
//! markup — a flattened PNG (the strokes burned in) + a sidecar JSON listing
//! every annotation in normalized 0–1 coords — written to `~/.gentle-eye/redpen/`.
//! The agent ingests it via `gentle-eye redpen-list` / `redpen-analyze`.
//! Enter = save+quit, Esc = cancel.

use eframe::egui::{self, Color32, Pos2, Rect, Sense, Stroke};
use gentle_eye::target::crop::crop_bgra;
use gentle_eye::target::model::PixelRect;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

/// Markup palette: index → (name, RGB). Switch with number keys 1-4 or the bar.
const PALETTE: [(&str, [u8; 3]); 4] = [
    ("red", [224, 36, 27]),
    ("blue", [37, 99, 235]),
    ("green", [22, 163, 74]),
    ("yellow", [202, 138, 4]),
];

#[derive(Clone, Copy, PartialEq)]
enum Tool {
    Pen,
    Arrow,
    Box,
}

/// One annotation in IMAGE-pixel space.
enum Shape {
    Pen(Vec<Pos2>),
    Arrow(Pos2, Pos2),
    Box(Rect),
}

struct Anno {
    shape: Shape,
    color: usize, // palette index
}

fn main() -> eframe::Result {
    // `redpen --list` / `--displays`: print the monitor catalogue and exit (no GUI).
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

struct RedpenApp {
    rgba: Vec<u8>,
    iw: usize,
    ih: usize,
    texture: Option<egui::TextureHandle>,
    annos: Vec<Anno>,
    tool: Tool,
    color: usize,
    pen_pts: Vec<Pos2>,       // accumulating freehand stroke (image space)
    drag_start: Option<Pos2>, // arrow/box start (image space)
    status: String,
}

impl RedpenApp {
    fn new(rgba: Vec<u8>, iw: usize, ih: usize) -> Self {
        Self {
            rgba,
            iw,
            ih,
            texture: None,
            annos: Vec::new(),
            tool: Tool::Pen,
            color: 0,
            pen_pts: Vec::new(),
            drag_start: None,
            status: format!(
                "{iw}×{ih} — Pen (P) / Arrow (A) / Box (B); colors 1-4. Enter saves, Esc cancels."
            ),
        }
    }

    fn color32(idx: usize) -> Color32 {
        let [r, g, b] = PALETTE[idx].1;
        Color32::from_rgb(r, g, b)
    }

    /// Flatten markup into a PNG + write a sidecar of all annotations (normalized).
    fn save(&mut self) {
        if self.annos.is_empty() {
            self.status = "nothing to save (draw something first)".into();
            return;
        }
        let dir = std::path::Path::new(&std::env::var("HOME").unwrap_or_default())
            .join(".gentle-eye/redpen");
        let _ = std::fs::create_dir_all(&dir);
        let ts = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        let png_path = dir.join(format!("{ts}.png"));
        let json_path = dir.join(format!("{ts}.json"));

        // Rasterize markup over a copy of the capture (what Gemini sees).
        if let Some(mut img) =
            image::RgbaImage::from_raw(self.iw as u32, self.ih as u32, self.rgba.clone())
        {
            for a in &self.annos {
                let [r, g, b] = PALETTE[a.color].1;
                let col = image::Rgba([r, g, b, 255]);
                match &a.shape {
                    Shape::Pen(pts) => {
                        for seg in pts.windows(2) {
                            draw_thick_line(&mut img, seg[0], seg[1], col, 3);
                        }
                    }
                    Shape::Arrow(from, to) => draw_arrow(&mut img, *from, *to, col),
                    Shape::Box(r) => {
                        let c = [
                            r.min,
                            egui::pos2(r.max.x, r.min.y),
                            r.max,
                            egui::pos2(r.min.x, r.max.y),
                        ];
                        for i in 0..4 {
                            draw_thick_line(&mut img, c[i], c[(i + 1) % 4], col, 3);
                        }
                    }
                }
            }
            let _ = img.save(&png_path);
        }

        // Sidecar — normalized annotations: the spatial-intent payload for the LLM.
        let (sw, sh) = (self.iw as f64, self.ih as f64);
        let nx = |p: Pos2| -> [f64; 2] { [p.x as f64 / sw, p.y as f64 / sh] };
        let entries: Vec<_> = self
            .annos
            .iter()
            .map(|a| {
                let color = PALETTE[a.color].0;
                match &a.shape {
                    Shape::Pen(pts) => serde_json::json!({
                        "type": "pen", "color": color,
                        "points": pts.iter().map(|p| nx(*p)).collect::<Vec<_>>(),
                    }),
                    Shape::Arrow(from, to) => serde_json::json!({
                        "type": "arrow", "color": color, "from": nx(*from), "to": nx(*to),
                    }),
                    Shape::Box(r) => serde_json::json!({
                        "type": "box", "color": color,
                        "rect": [r.min.x as f64 / sw, r.min.y as f64 / sh,
                                 r.width() as f64 / sw, r.height() as f64 / sh],
                    }),
                }
            })
            .collect();
        let _ = std::fs::write(
            &json_path,
            serde_json::to_string_pretty(&serde_json::json!({
                "image": png_path.to_string_lossy(),
                "size": [self.iw, self.ih],
                "annotations": entries,
            }))
            .unwrap_or_default(),
        );

        // Files flushed; exit to close the window reliably.
        eprintln!("saved {} annotation(s) → {}", self.annos.len(), png_path.display());
        std::process::exit(0);
    }
}

impl eframe::App for RedpenApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        if self.texture.is_none() {
            let ci = egui::ColorImage::from_rgba_unmultiplied([self.iw, self.ih], &self.rgba);
            self.texture = Some(ctx.load_texture("shot", ci, egui::TextureOptions::default()));
        }

        // Esc = cancel; Enter = save. Tool/color shortcuts.
        if ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
            std::process::exit(0);
        }
        let mut save_now = ctx.input(|i| i.key_pressed(egui::Key::Enter));
        ctx.input(|i| {
            if i.key_pressed(egui::Key::Num1) {
                self.color = 0;
            }
            if i.key_pressed(egui::Key::Num2) {
                self.color = 1;
            }
            if i.key_pressed(egui::Key::Num3) {
                self.color = 2;
            }
            if i.key_pressed(egui::Key::Num4) {
                self.color = 3;
            }
            if i.key_pressed(egui::Key::P) {
                self.tool = Tool::Pen;
            }
            if i.key_pressed(egui::Key::A) {
                self.tool = Tool::Arrow;
            }
            if i.key_pressed(egui::Key::B) {
                self.tool = Tool::Box;
            }
        });

        egui::TopBottomPanel::bottom("bar").show(ctx, |ui| {
            ui.horizontal_wrapped(|ui| {
                if ui.button("Save & Quit (Enter)").clicked() {
                    save_now = true;
                }
                if ui.button("Undo (last)").clicked() {
                    self.annos.pop();
                }
                if ui.button("Cancel (Esc)").clicked() {
                    std::process::exit(0);
                }
                ui.separator();
                ui.label("Tool:");
                ui.selectable_value(&mut self.tool, Tool::Pen, "Pen (P)");
                ui.selectable_value(&mut self.tool, Tool::Arrow, "Arrow (A)");
                ui.selectable_value(&mut self.tool, Tool::Box, "Box (B)");
                ui.separator();
                ui.label("Color:");
                for (i, (name, _)) in PALETTE.iter().enumerate() {
                    let mut b = egui::Button::new(*name).fill(Self::color32(i));
                    if self.color == i {
                        b = b.stroke(Stroke::new(2.0, Color32::WHITE));
                    }
                    if ui.add(b).clicked() {
                        self.color = i;
                    }
                }
            });
            ui.label(&self.status);
        });

        egui::CentralPanel::default().show(ctx, |ui| {
            let (resp, painter) = ui.allocate_painter(ui.available_size(), Sense::click_and_drag());
            let panel = resp.rect;
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
            match self.tool {
                Tool::Pen => {
                    if resp.drag_started() {
                        self.pen_pts.clear();
                    }
                    if resp.dragged() {
                        if let Some(p) = resp.interact_pointer_pos() {
                            self.pen_pts.push(to_img(p));
                        }
                    }
                    if resp.drag_stopped() {
                        if self.pen_pts.len() > 1 {
                            self.annos.push(Anno {
                                shape: Shape::Pen(std::mem::take(&mut self.pen_pts)),
                                color: self.color,
                            });
                        }
                        self.pen_pts.clear();
                    }
                }
                Tool::Arrow | Tool::Box => {
                    if resp.drag_started() {
                        self.drag_start = resp.interact_pointer_pos().map(to_img);
                    }
                    if resp.drag_stopped() {
                        if let (Some(s), Some(e)) =
                            (self.drag_start, resp.interact_pointer_pos().map(to_img))
                        {
                            if (s - e).length() > 4.0 {
                                let shape = if self.tool == Tool::Arrow {
                                    Shape::Arrow(s, e)
                                } else {
                                    Shape::Box(Rect::from_two_pos(s, e))
                                };
                                self.annos.push(Anno { shape, color: self.color });
                            }
                        }
                        self.drag_start = None;
                    }
                }
            }

            // Render committed annotations (image-px → screen).
            for a in &self.annos {
                let stroke = Stroke::new(2.5, Self::color32(a.color));
                match &a.shape {
                    Shape::Pen(pts) => {
                        let scr: Vec<Pos2> = pts.iter().map(|p| to_screen(*p)).collect();
                        painter.add(egui::Shape::line(scr, stroke));
                    }
                    Shape::Arrow(f, t) => paint_arrow(&painter, to_screen(*f), to_screen(*t), stroke),
                    Shape::Box(r) => {
                        let sr = Rect::from_two_pos(to_screen(r.min), to_screen(r.max));
                        painter.rect_stroke(sr, egui::Rounding::ZERO, stroke);
                    }
                }
            }

            // Live draft of the in-progress stroke.
            let stroke = Stroke::new(2.5, Self::color32(self.color));
            match self.tool {
                Tool::Pen if !self.pen_pts.is_empty() => {
                    let scr: Vec<Pos2> = self.pen_pts.iter().map(|p| to_screen(*p)).collect();
                    painter.add(egui::Shape::line(scr, stroke));
                }
                Tool::Arrow | Tool::Box => {
                    if let (Some(s), Some(cur)) =
                        (self.drag_start, resp.interact_pointer_pos().map(to_img))
                    {
                        if self.tool == Tool::Arrow {
                            paint_arrow(&painter, to_screen(s), to_screen(cur), stroke);
                        } else {
                            painter.rect_stroke(
                                Rect::from_two_pos(to_screen(s), to_screen(cur)),
                                egui::Rounding::ZERO,
                                stroke,
                            );
                        }
                    }
                }
                _ => {}
            }
        });

        // Save after the panels so we don't alias `self` inside the closures.
        if save_now {
            self.save();
        }
    }
}

/// Paint an arrow (line + two barbs) in egui screen space.
fn paint_arrow(painter: &egui::Painter, from: Pos2, to: Pos2, stroke: Stroke) {
    painter.line_segment([from, to], stroke);
    let dir = to - from;
    let len = dir.length().max(1.0);
    let u = dir / len;
    let head = (len * 0.2).clamp(8.0, 28.0);
    let back = to - u * head;
    let perp = egui::vec2(-u.y, u.x) * (head * 0.5);
    painter.line_segment([to, back + perp], stroke);
    painter.line_segment([to, back - perp], stroke);
}

/// Draw a `thick`-px line into the PNG by stamping the 1-px line over a small box.
fn draw_thick_line(img: &mut image::RgbaImage, a: Pos2, b: Pos2, col: image::Rgba<u8>, thick: i32) {
    use imageproc::drawing::draw_line_segment_mut;
    let r = thick / 2;
    for dx in -r..=r {
        for dy in -r..=r {
            draw_line_segment_mut(
                img,
                (a.x + dx as f32, a.y + dy as f32),
                (b.x + dx as f32, b.y + dy as f32),
                col,
            );
        }
    }
}

/// Draw an arrow (shaft + two barbs) into the PNG.
fn draw_arrow(img: &mut image::RgbaImage, from: Pos2, to: Pos2, col: image::Rgba<u8>) {
    draw_thick_line(img, from, to, col, 3);
    let dir = to - from;
    let len = dir.length().max(1.0);
    let u = dir / len;
    let head = (len * 0.2).clamp(8.0, 28.0);
    let back = to - u * head;
    let perp = egui::vec2(-u.y, u.x) * (head * 0.5);
    draw_thick_line(img, to, back + perp, col, 3);
    draw_thick_line(img, to, back - perp, col, 3);
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
