//! Vectorize — Desktop GUI Application (Rust / egui)
//!
//! Single-window app for vectorizing raster images to SVG.
//! Trilithium / brushed-metal theme matching the Stitcher UI.

#![windows_subsystem = "windows"]

mod theme;

use eframe::egui;
use image::DynamicImage;
use std::path::PathBuf;
use std::sync::mpsc;
use theme::*;
use vectorize_core::quality::{Mode, QualitySettings};
use vectorize_core::{Engine, LayerMode, VectorizeConfig};

// ── UI Config (loaded from ui_config.json) ──────────────────────────────────

#[derive(Debug, Clone, serde::Deserialize)]
struct UiConfig {
    window_width: f32,
    window_height: f32,
    window_min_width: f32,
    window_min_height: f32,
    titlebar_height: f32,
    toolbar_height: f32,
    statusbar_height: f32,
    progress_height: f32,
    side_panel_width: f32,
    side_panel_margin_x: f32,
    side_panel_margin_y: f32,
    inset_padding_x: f32,
    inset_padding_y: f32,
    section_spacing: f32,
    section_header_font_size: f32,
    log_box_height: f32,
    vectorize_row_height: f32,
    vectorize_button_width: f32,
    slider_label_width: f32,
    slider_font_size: f32,
    slider_height: f32,
    log_font_size: f32,
    status_font_size: f32,
}

impl Default for UiConfig {
    fn default() -> Self {
        Self {
            window_width: 1100.0,
            window_height: 930.0,
            window_min_width: 800.0,
            window_min_height: 600.0,
            titlebar_height: 22.0,
            toolbar_height: 26.0,
            statusbar_height: 20.0,
            progress_height: 12.0,
            side_panel_width: 280.0,
            side_panel_margin_x: 10.0,
            side_panel_margin_y: 4.0,
            inset_padding_x: 6.0,
            inset_padding_y: 4.0,
            section_spacing: 6.0,
            section_header_font_size: 12.0,
            log_box_height: 110.0,
            vectorize_row_height: 20.0,
            vectorize_button_width: 90.0,
            slider_label_width: 110.0,
            slider_font_size: 9.0,
            slider_height: 16.0,
            log_font_size: 9.0,
            status_font_size: 10.0,
        }
    }
}

fn load_ui_config() -> UiConfig {
    // Look for config next to the executable first, then in crate root
    let paths = [
        std::env::current_exe()
            .ok()
            .and_then(|p| p.parent().map(|d| d.join("ui_config.json"))),
        Some(PathBuf::from("ui_config.json")),
        Some(PathBuf::from("crates/vectorize-gui/ui_config.json")),
    ];
    for path in paths.iter().flatten() {
        if let Ok(text) = std::fs::read_to_string(path) {
            match serde_json::from_str::<UiConfig>(&text) {
                Ok(cfg) => {
                    eprintln!("Loaded UI config from {}", path.display());
                    return cfg;
                }
                Err(e) => {
                    eprintln!("Warning: failed to parse {}: {}", path.display(), e);
                }
            }
        }
    }
    eprintln!("Using default UI config (no ui_config.json found)");
    UiConfig::default()
}

// ── Constants ────────────────────────────────────────────────────────────────

const MODE_LABELS: [&str; 5] = ["Logo", "Illust", "Photo", "HiFi", "Sketch"];
const MODE_VALUES: [Mode; 5] = [
    Mode::Logo,
    Mode::Illustration,
    Mode::Photo,
    Mode::HighFidelity,
    Mode::Sketch,
];

const ENGINE_LABELS: [&str; 3] = ["VTracer", "Hybrid", "Native"];
const ENGINE_VALUES: [Engine; 3] = [Engine::Vtracer, Engine::Hybrid, Engine::Native];

const LAYER_LABELS: [&str; 2] = ["Stacked", "Cutout"];
const LAYER_VALUES: [LayerMode; 2] = [LayerMode::Stacked, LayerMode::Cutout];

// ── Job channel message ──────────────────────────────────────────────────────

enum JobResult {
    SvgDone(String),
    Error(String),
    Log(String),
    Progress(f32, String),
}

// ── Persistent settings ──────────────────────────────────────────────────────

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
struct SavedSettings {
    preset_idx: usize,
    engine_idx: usize,
    layer_idx: usize,
    detect_shapes: bool,
    extract_lines: bool,
    merge_paths: bool,
    snap_curves_to_lines: bool,
    create_fills: bool,
    create_strokes: bool,
    flatten_background: bool,
    skip_transparent: bool,
    simplify_method_idx: usize,
    tones_per_hue: u8,
    min_area: u32,
    edge_smoothing: f32,
    color_threshold: f32,
    anchor_density: f32,
    color_detail: f32,
    path_precision: f32,
    curve_smoothness: f32,
    noise_filter: f32,
    gradient_layers: f32,
    shadow_detail: f32,
    midtone_detail: f32,
    highlight_detail: f32,
}

fn settings_path() -> PathBuf {
    let dir = dirs::config_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("vectorize");
    let _ = std::fs::create_dir_all(&dir);
    dir.join("settings.json")
}

fn load_settings() -> Option<SavedSettings> {
    let path = settings_path();
    let data = std::fs::read_to_string(&path).ok()?;
    serde_json::from_str(&data).ok()
}

fn save_settings(s: &SavedSettings) {
    if let Ok(json) = serde_json::to_string_pretty(s) {
        let _ = std::fs::write(settings_path(), json);
    }
}

// ── Tab ──────────────────────────────────────────────────────────────────────

#[derive(Clone, Copy, PartialEq)]
enum Tab {
    Input,
    Output,
}

// ── App state ────────────────────────────────────────────────────────────────

struct VectorizeApp {
    // Image
    input_path: Option<PathBuf>,
    input_image: Option<DynamicImage>,
    input_texture: Option<egui::TextureHandle>,
    input_width: u32,
    input_height: u32,

    // SVG result
    svg_string: Option<String>,
    output_texture: Option<egui::TextureHandle>,

    // Settings
    preset_idx: usize,
    engine_idx: usize,
    layer_idx: usize,
    detect_shapes: bool,
    extract_lines: bool,
    merge_paths: bool,
    snap_curves_to_lines: bool,
    create_fills: bool,
    create_strokes: bool,
    flatten_background: bool,
    skip_transparent: bool,
    simplify_method_idx: usize, // 0=KurboBezier, 1=VisvalingamWhyatt
    tones_per_hue: u8,
    color_count: u32,
    min_area: u32,
    edge_smoothing: f32,
    color_threshold: f32,
    anchor_density: f32,
    color_detail: f32,
    path_precision: f32,
    curve_smoothness: f32,
    noise_filter: f32,
    gradient_layers: f32,
    shadow_detail: f32,
    midtone_detail: f32,
    highlight_detail: f32,
    output_path: String,

    // UI state
    active_tab: Tab,
    advanced_expanded: bool,
    log_entries: Vec<String>,
    progress: f32,
    progress_text: String,
    busy: bool,

    // Zoom / pan (shared for both tabs)
    input_zoom: f32,
    input_offset: egui::Vec2,
    output_zoom: f32,
    output_offset: egui::Vec2,

    // Background job
    job_rx: Option<mpsc::Receiver<JobResult>>,
    progress_state: Option<std::sync::Arc<vectorize_core::ProgressState>>,

    // UI layout config (from ui_config.json)
    ui_cfg: UiConfig,

}

impl Default for VectorizeApp {
    fn default() -> Self {
        let recipe = Mode::Logo.recipe();
        let mut app = Self {
            input_path: None,
            input_image: None,
            input_texture: None,
            input_width: 0,
            input_height: 0,

            svg_string: None,
            output_texture: None,

            preset_idx: 0, // Logo
            engine_idx: ENGINE_VALUES.iter().position(|e| *e == recipe.preferred_engine).unwrap_or(0),
            layer_idx: match recipe.layer_mode { LayerMode::Cutout => 1, _ => 0 },
            detect_shapes: recipe.detect_shapes,
            extract_lines: recipe.extract_lines,
            merge_paths: recipe.merge_paths,
            snap_curves_to_lines: false,
            create_fills: true,
            create_strokes: false,
            flatten_background: recipe.flatten_background,
            skip_transparent: recipe.skip_transparent,
            simplify_method_idx: match recipe.simplify_method {
                vectorize_core::SimplifyMethod::VisvalingamWhyatt => 1,
                _ => 0,
            },
            tones_per_hue: recipe.tones_per_hue,
            color_count: 0,
            min_area: recipe.min_area,
            edge_smoothing: recipe.edge_smoothing as f32,
            color_threshold: recipe.color_threshold as f32,
            anchor_density: recipe.anchor_density as f32,
            color_detail: recipe.color_detail,
            path_precision: recipe.path_precision,
            curve_smoothness: recipe.curve_smoothness,
            noise_filter: recipe.noise_filter,
            gradient_layers: recipe.gradient_layers,
            shadow_detail: 100.0,
            midtone_detail: 100.0,
            highlight_detail: 100.0,
            output_path: dirs_or_default(),

            active_tab: Tab::Input,
            advanced_expanded: false,
            log_entries: Vec::new(),
            progress: 0.0,
            progress_text: "Ready".into(),
            busy: false,

            input_zoom: 1.0,
            input_offset: egui::Vec2::ZERO,
            output_zoom: 1.0,
            output_offset: egui::Vec2::ZERO,

            job_rx: None,
            progress_state: None,

            ui_cfg: load_ui_config(),
        };

        // Restore saved settings from previous session
        if let Some(saved) = load_settings() {
            app.apply_saved_settings(&saved);
        }

        app
    }
}

fn dirs_or_default() -> String {
    if let Some(home) = std::env::var_os("USERPROFILE").or_else(|| std::env::var_os("HOME")) {
        let p = PathBuf::from(home).join("Downloads");
        if p.exists() {
            return p.to_string_lossy().into();
        }
    }
    ".".into()
}

// ── Helpers ──────────────────────────────────────────────────────────────────

impl VectorizeApp {
    fn to_saved_settings(&self) -> SavedSettings {
        SavedSettings {
            preset_idx: self.preset_idx,
            engine_idx: self.engine_idx,
            layer_idx: self.layer_idx,
            detect_shapes: self.detect_shapes,
            extract_lines: self.extract_lines,
            merge_paths: self.merge_paths,
            snap_curves_to_lines: self.snap_curves_to_lines,
            create_fills: self.create_fills,
            create_strokes: self.create_strokes,
            flatten_background: self.flatten_background,
            skip_transparent: self.skip_transparent,
            simplify_method_idx: self.simplify_method_idx,
            tones_per_hue: self.tones_per_hue,
            min_area: self.min_area,
            edge_smoothing: self.edge_smoothing,
            color_threshold: self.color_threshold,
            anchor_density: self.anchor_density,
            color_detail: self.color_detail,
            path_precision: self.path_precision,
            curve_smoothness: self.curve_smoothness,
            noise_filter: self.noise_filter,
            gradient_layers: self.gradient_layers,
            shadow_detail: self.shadow_detail,
            midtone_detail: self.midtone_detail,
            highlight_detail: self.highlight_detail,
        }
    }

    fn apply_saved_settings(&mut self, s: &SavedSettings) {
        self.preset_idx = s.preset_idx.min(MODE_VALUES.len() - 1);
        self.engine_idx = s.engine_idx.min(ENGINE_VALUES.len() - 1);
        self.layer_idx = s.layer_idx.min(LAYER_VALUES.len() - 1);
        self.detect_shapes = s.detect_shapes;
        self.extract_lines = s.extract_lines;
        self.merge_paths = s.merge_paths;
        self.snap_curves_to_lines = s.snap_curves_to_lines;
        self.create_fills = s.create_fills;
        self.create_strokes = s.create_strokes;
        self.flatten_background = s.flatten_background;
        self.skip_transparent = s.skip_transparent;
        self.simplify_method_idx = s.simplify_method_idx.min(1);
        self.tones_per_hue = s.tones_per_hue;
        self.min_area = s.min_area;
        self.edge_smoothing = s.edge_smoothing;
        self.color_threshold = s.color_threshold;
        self.anchor_density = s.anchor_density;
        self.color_detail = s.color_detail;
        self.path_precision = s.path_precision;
        self.curve_smoothness = s.curve_smoothness;
        self.noise_filter = s.noise_filter;
        self.gradient_layers = s.gradient_layers;
        self.shadow_detail = s.shadow_detail;
        self.midtone_detail = s.midtone_detail;
        self.highlight_detail = s.highlight_detail;
    }

    fn log(&mut self, msg: &str) {
        self.log_entries.push(msg.to_string());
    }

    fn set_progress(&mut self, v: f32, t: &str) {
        self.progress = v.clamp(0.0, 1.0);
        self.progress_text = t.into();
    }

    /// Apply a mode — sets quality sliders AND algorithm/feature selections.
    fn apply_mode(&mut self, idx: usize) {
        self.preset_idx = idx;
        let mode = MODE_VALUES[idx];
        let recipe = mode.recipe();

        // Quality sliders
        self.color_detail = recipe.color_detail;
        self.path_precision = recipe.path_precision;
        self.curve_smoothness = recipe.curve_smoothness;
        self.noise_filter = recipe.noise_filter;
        self.gradient_layers = recipe.gradient_layers;
        self.shadow_detail = 100.0;
        self.midtone_detail = 100.0;
        self.highlight_detail = 100.0;

        // Algorithm selection (keep user's engine choice — don't override)
        self.simplify_method_idx = match recipe.simplify_method {
            vectorize_core::SimplifyMethod::VisvalingamWhyatt => 1,
            _ => 0,
        };

        // Feature flags
        self.detect_shapes = recipe.detect_shapes;
        self.extract_lines = recipe.extract_lines;
        self.merge_paths = recipe.merge_paths;
        self.flatten_background = recipe.flatten_background;
        self.skip_transparent = recipe.skip_transparent;

        // Pipeline parameters
        self.layer_idx = match recipe.layer_mode {
            LayerMode::Cutout => 1,
            _ => 0,
        };
        self.edge_smoothing = recipe.edge_smoothing as f32;
        self.color_threshold = recipe.color_threshold as f32;
        self.anchor_density = recipe.anchor_density as f32;
        self.tones_per_hue = recipe.tones_per_hue;
        self.min_area = recipe.min_area;
    }

    fn preset_name(&self) -> &str {
        MODE_LABELS[self.preset_idx]
    }

    fn open_image(&mut self, path: PathBuf, ctx: &egui::Context) {
        let name = path.file_name().unwrap_or_default().to_string_lossy().to_string();
        self.log(&format!("Opening {name}..."));
        match image::open(&path) {
            Ok(img) => {
                self.input_width = img.width();
                self.input_height = img.height();
                self.log(&format!(
                    "  Loaded {}x{} ({} bytes)",
                    self.input_width,
                    self.input_height,
                    img.as_bytes().len()
                ));

                // Convert to RGBA texture
                let rgba = img.to_rgba8();
                let size = [rgba.width() as usize, rgba.height() as usize];
                let color_image =
                    egui::ColorImage::from_rgba_unmultiplied(size, rgba.as_raw());
                self.input_texture = Some(ctx.load_texture(
                    "input_preview",
                    color_image,
                    egui::TextureOptions::LINEAR,
                ));

                self.input_image = Some(img);
                self.input_path = Some(path);
                self.active_tab = Tab::Input;
                self.input_zoom = 1.0;
                self.input_offset = egui::Vec2::ZERO;
                self.svg_string = None;
                self.output_texture = None;
            }
            Err(e) => self.log(&format!("  ERROR: {e}")),
        }
    }

    fn build_config(&self) -> VectorizeConfig {
        let quality = QualitySettings::from_mode(MODE_VALUES[self.preset_idx])
            .with_color_detail(self.color_detail)
            .with_path_precision(self.path_precision)
            .with_curve_smoothness(self.curve_smoothness)
            .with_noise_filter(self.noise_filter)
            .with_gradient_layers(self.gradient_layers)
            .with_shadow_detail(self.shadow_detail)
            .with_midtone_detail(self.midtone_detail)
            .with_highlight_detail(self.highlight_detail);

        let simplify_method = match self.simplify_method_idx {
            1 => vectorize_core::SimplifyMethod::VisvalingamWhyatt,
            _ => vectorize_core::SimplifyMethod::KurboBezier,
        };
        VectorizeConfig {
            color_count: 0, // Always auto — Color Detail drives the count
            min_area: self.min_area,
            engine: ENGINE_VALUES[self.engine_idx],
            layer_mode: LAYER_VALUES[self.layer_idx],
            detect_shapes: self.detect_shapes,
            extract_lines: self.extract_lines,
            merge_paths: self.merge_paths,
            flatten_background: self.flatten_background,
            skip_transparent: self.skip_transparent,
            simplify_method,
            tones_per_hue: self.tones_per_hue,
            edge_smoothing: self.edge_smoothing as f64,
            color_threshold: self.color_threshold as f64,
            anchor_density: self.anchor_density as f64,
            snap_curves_to_lines: self.snap_curves_to_lines,
            create_fills: self.create_fills,
            create_strokes: self.create_strokes,
            quality,
            mode: MODE_VALUES[self.preset_idx],
            ..Default::default()
        }
    }

    fn start_vectorize(&mut self) {
        let Some(img) = self.input_image.clone() else { return };
        if self.busy { return; }
        self.busy = true;
        self.set_progress(0.0, "Vectorizing...");
        self.log("Starting vectorization...");

        let config = self.build_config();
        let (tx, rx) = mpsc::channel();
        self.job_rx = Some(rx);

        // Shared progress state — vectorizer writes, GUI reads each frame.
        let progress_state = std::sync::Arc::new(vectorize_core::ProgressState::new());
        self.progress_state = Some(std::sync::Arc::clone(&progress_state));

        std::thread::spawn(move || {
            let _ = tx.send(JobResult::Log(format!(
                "  Engine: {:?}  Mode: {:?}",
                config.engine, config.mode
            )));

            // Catch any panics that escape the vectorizer's internal catch_unwind guards.
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                vectorize_core::vectorize_with_progress(&img, &config, &progress_state)
            }));
            match result {
                Ok(Ok(svg)) => {
                    let _ = tx.send(JobResult::Log(format!(
                        "  SVG generated: {} bytes",
                        svg.len()
                    )));
                    let _ = tx.send(JobResult::Progress(1.0, "Done!".into()));
                    let _ = tx.send(JobResult::SvgDone(svg));
                }
                Ok(Err(e)) => {
                    let _ = tx.send(JobResult::Error(format!("{e}")));
                }
                Err(_) => {
                    let _ = tx.send(JobResult::Error(
                        "Vectorization crashed — try different settings or a smaller image".into(),
                    ));
                }
            }
        });
    }

    fn cancel_vectorize(&mut self) {
        // Signal the vectorizer to stop.
        if let Some(state) = &self.progress_state {
            state.cancelled.store(true, std::sync::atomic::Ordering::Relaxed);
        }
        // Drop the receiver so the background thread's sends fail silently.
        self.job_rx = None;
        self.progress_state = None;
        self.busy = false;
        self.set_progress(0.0, "Cancelled");
        self.log("Vectorization cancelled.");
    }

    fn clear_all(&mut self) {
        // Cancel any running job.
        self.job_rx = None;
        self.progress_state = None;
        self.busy = false;

        // Clear images and output.
        self.input_path = None;
        self.input_image = None;
        self.input_texture = None;
        self.input_width = 0;
        self.input_height = 0;
        self.svg_string = None;
        self.output_texture = None;

        // Reset settings to defaults via mode system.
        self.apply_mode(0); // Logo mode

        // Reset UI state.
        self.active_tab = Tab::Input;
        self.set_progress(0.0, "Ready");
        self.log_entries.clear();
        self.input_zoom = 1.0;
        self.input_offset = egui::Vec2::ZERO;
        self.output_zoom = 1.0;
        self.output_offset = egui::Vec2::ZERO;
    }

    fn export_svg(&mut self) {
        let Some(svg) = &self.svg_string else { return };
        if let Some(path) = rfd::FileDialog::new()
            .set_title("Export SVG")
            .set_directory(&self.output_path)
            .add_filter("SVG", &["svg"])
            .save_file()
        {
            match std::fs::write(&path, svg) {
                Ok(_) => self.log(&format!("Exported: {}", path.display())),
                Err(e) => self.log(&format!("Export error: {e}")),
            }
        }
    }

    fn render_svg_to_texture(&mut self, svg: &str, ctx: &egui::Context) {
        let opt = usvg::Options::default();
        match usvg::Tree::from_str(svg, &opt) {
            Ok(tree) => {
                let tree_size = tree.size();
                let w = tree_size.width() as u32;
                let h = tree_size.height() as u32;
                // Cap preview size to avoid huge allocations
                let max_dim = 4096u32;
                let (pw, ph) = if w > max_dim || h > max_dim {
                    let scale = max_dim as f32 / w.max(h) as f32;
                    ((w as f32 * scale) as u32, (h as f32 * scale) as u32)
                } else {
                    (w.max(1), h.max(1))
                };

                if let Some(mut pixmap) = resvg::tiny_skia::Pixmap::new(pw, ph) {
                    pixmap.fill(resvg::tiny_skia::Color::WHITE);
                    let sx = pw as f32 / tree_size.width();
                    let sy = ph as f32 / tree_size.height();
                    resvg::render(
                        &tree,
                        resvg::tiny_skia::Transform::from_scale(sx, sy),
                        &mut pixmap.as_mut(),
                    );
                    let rgba = pixmap.data();
                    let color_image = egui::ColorImage::from_rgba_unmultiplied(
                        [pw as usize, ph as usize],
                        rgba,
                    );
                    self.output_texture = Some(ctx.load_texture(
                        "output_preview",
                        color_image,
                        egui::TextureOptions::LINEAR,
                    ));
                    self.log(&format!("  Preview rendered: {pw}x{ph}"));
                } else {
                    self.log("  Warning: Could not create pixmap for preview.");
                }
            }
            Err(e) => self.log(&format!("  SVG parse error: {e}")),
        }
    }

    fn poll_jobs(&mut self, ctx: &egui::Context) {
        let messages: Vec<JobResult> = if let Some(rx) = &self.job_rx {
            let mut msgs = Vec::new();
            while let Ok(msg) = rx.try_recv() {
                msgs.push(msg);
            }
            msgs
        } else {
            return;
        };

        for msg in messages {
            match msg {
                JobResult::Log(s) => self.log(&s),
                JobResult::Progress(v, s) => self.set_progress(v, &s),
                JobResult::Error(e) => {
                    self.log(&format!("ERROR: {e}"));
                    self.busy = false;
                    self.progress_state = None;
                }
                JobResult::SvgDone(svg) => {
                    self.render_svg_to_texture(&svg, ctx);
                    self.svg_string = Some(svg);
                    self.active_tab = Tab::Output;
                    self.output_zoom = 1.0;
                    self.output_offset = egui::Vec2::ZERO;
                    self.busy = false;
                    self.progress_state = None;
                }
            }
        }

        // Poll shared progress state from the vectorizer (lock-free).
        if let Some(state) = &self.progress_state {
            use std::sync::atomic::Ordering::Relaxed;
            let current = state.current.load(Relaxed);
            let total = state.total.load(Relaxed);
            let stage = state.stage_name();
            let msg = if total > 1 {
                format!("{} {}/{}", stage, current, total)
            } else {
                stage.to_string()
            };
            let frac = if total > 0 {
                0.1 + 0.8 * (current as f32 / total as f32)
            } else {
                0.1
            };
            self.set_progress(frac, &msg);
        }
    }
}

// ── Slider helper ────────────────────────────────────────────────────────────

fn labeled_slider(ui: &mut egui::Ui, label: &str, value: &mut f32) {
    labeled_slider_color(ui, label, value, 0.0, 200.0, "", RETRO_AMBER);
}

fn labeled_slider_range(ui: &mut egui::Ui, label: &str, value: &mut f32, min: f32, max: f32) {
    labeled_slider_color(ui, label, value, min, max, "", RETRO_AMBER);
}

/// Core slider with tooltip support (uses default accent color).
fn labeled_slider_tip(ui: &mut egui::Ui, label: &str, value: &mut f32, min: f32, max: f32, tip: &str) {
    labeled_slider_color(ui, label, value, min, max, tip, RETRO_AMBER);
}

/// Slider with per-slider accent color.
/// Paints a colored fill bar over the slider track to show the accent.
fn labeled_slider_color(ui: &mut egui::Ui, label: &str, value: &mut f32, min: f32, max: f32, tip: &str, accent: egui::Color32) {
    labeled_slider_color_enabled(ui, label, value, min, max, tip, accent, true);
}

fn labeled_slider_color_enabled(ui: &mut egui::Ui, label: &str, value: &mut f32, min: f32, max: f32, tip: &str, accent: egui::Color32, enabled: bool) {
    let dim = if enabled { accent } else { egui::Color32::from_rgb(0x55, 0x55, 0x50) };
    let resp = ui.horizontal(|ui| {
        ui.set_enabled(enabled);
        let label_width = 110.0;
        let (label_rect, label_resp) =
            ui.allocate_exact_size(egui::vec2(label_width, 16.0), egui::Sense::hover());
        let font = egui::FontId::monospace(9.0);
        let text_pos = egui::pos2(label_rect.min.x, label_rect.center().y - 5.0);
        embossed_text(ui, text_pos, label, font, dim);
        let slider_w = ui.available_width() - 38.0;

        let slider_resp = ui.add_sized(
            [slider_w.max(40.0), 16.0],
            egui::Slider::new(value, min..=max).show_value(false),
        );

        // Paint colored fill over the slider track + knob
        let sr = slider_resp.rect;
        let frac = ((*value - min) / (max - min)).clamp(0.0, 1.0);
        let track_margin = 5.0;
        let track_w = sr.width() - track_margin * 2.0;
        let fill_w = track_w * frac;
        let fill_rect = egui::Rect::from_min_size(
            egui::pos2(sr.min.x + track_margin, sr.center().y - 2.0),
            egui::vec2(fill_w, 4.0),
        );
        ui.painter().rect_filled(fill_rect, 0.0, dim);
        // Knob at the current position
        let knob_x = sr.min.x + track_margin + fill_w;
        let knob_rect = egui::Rect::from_center_size(
            egui::pos2(knob_x, sr.center().y),
            egui::vec2(6.0, sr.height() - 2.0),
        );
        ui.painter().rect_filled(knob_rect, 0.0, dim);

        ui.label(
            egui::RichText::new(format!("{:5.1}", *value))
                .monospace().size(9.0).color(dim),
        );
        label_resp | slider_resp
    });
    if !tip.is_empty() {
        resp.inner.on_hover_text_at_pointer(tip);
    }
}

/// Darken a color by a factor (0.0 = black, 1.0 = unchanged).
fn darken(c: egui::Color32, factor: f32) -> egui::Color32 {
    egui::Color32::from_rgb(
        (c.r() as f32 * factor) as u8,
        (c.g() as f32 * factor) as u8,
        (c.b() as f32 * factor) as u8,
    )
}

/// Integer slider with tooltip support.
fn int_slider(ui: &mut egui::Ui, label: &str, value: &mut u32, min: u32, max: u32, zero_label: &str) {
    int_slider_color(ui, label, value, min, max, zero_label, "", RETRO_AMBER);
}

fn int_slider_tip(ui: &mut egui::Ui, label: &str, value: &mut u32, min: u32, max: u32, zero_label: &str, tip: &str) {
    int_slider_color(ui, label, value, min, max, zero_label, tip, RETRO_AMBER);
}

fn int_slider_color(ui: &mut egui::Ui, label: &str, value: &mut u32, min: u32, max: u32, zero_label: &str, tip: &str, accent: egui::Color32) {
    int_slider_color_enabled(ui, label, value, min, max, zero_label, tip, accent, true);
}

fn int_slider_color_enabled(ui: &mut egui::Ui, label: &str, value: &mut u32, min: u32, max: u32, zero_label: &str, tip: &str, accent: egui::Color32, enabled: bool) {
    let dim = if enabled { accent } else { egui::Color32::from_rgb(0x55, 0x55, 0x50) };
    let resp = ui.horizontal(|ui| {
        ui.set_enabled(enabled);
        let label_width = 110.0;
        let (label_rect, label_resp) =
            ui.allocate_exact_size(egui::vec2(label_width, 16.0), egui::Sense::hover());
        let font = egui::FontId::monospace(9.0);
        let text_pos = egui::pos2(label_rect.min.x, label_rect.center().y - 5.0);
        embossed_text(ui, text_pos, label, font, dim);

        let slider_w = ui.available_width() - 38.0;
        let mut v = *value as f32;
        let slider_resp = ui.add_sized(
            [slider_w.max(40.0), 16.0],
            egui::Slider::new(&mut v, min as f32..=max as f32)
                .step_by(1.0)
                .show_value(false),
        );
        *value = v as u32;

        // Paint colored fill over the slider track + knob
        let sr = slider_resp.rect;
        let range = (max - min).max(1) as f32;
        let frac = ((*value - min) as f32 / range).clamp(0.0, 1.0);
        let track_margin = 5.0;
        let track_w = sr.width() - track_margin * 2.0;
        let fill_w = track_w * frac;
        let fill_rect = egui::Rect::from_min_size(
            egui::pos2(sr.min.x + track_margin, sr.center().y - 2.0),
            egui::vec2(fill_w, 4.0),
        );
        ui.painter().rect_filled(fill_rect, 0.0, dim);
        let knob_x = sr.min.x + track_margin + fill_w;
        let knob_rect = egui::Rect::from_center_size(
            egui::pos2(knob_x, sr.center().y),
            egui::vec2(6.0, sr.height() - 2.0),
        );
        ui.painter().rect_filled(knob_rect, 0.0, dim);
        let display = if *value == 0 && !zero_label.is_empty() {
            zero_label.to_string()
        } else {
            format!("{}", *value)
        };
        ui.label(
            egui::RichText::new(format!("{:>4}", display))
                .monospace().size(9.0).color(dim),
        );
        label_resp | slider_resp
    });
    if !tip.is_empty() {
        resp.inner.on_hover_text_at_pointer(tip);
    }
}

// ── eframe::App ──────────────────────────────────────────────────────────────

impl eframe::App for VectorizeApp {
    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        save_settings(&self.to_saved_settings());
    }

    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.poll_jobs(ctx);
        if self.busy {
            ctx.request_repaint();
        }

        // ── Custom title bar (22px) ──────────────────────────────────────
        egui::TopBottomPanel::top("titlebar")
            .exact_height(self.ui_cfg.titlebar_height)
            .frame(
                egui::Frame::new()
                    .fill(BG_DARK)
                    .inner_margin(egui::Margin::same(0)),
            )
            .show(ctx, |ui| {
                let bar_rect = ui.max_rect();

                // Grip dots left
                for gy in (4..20).step_by(3) {
                    for gx in (4..56).step_by(4) {
                        let p = bar_rect.min + egui::vec2(gx as f32, gy as f32);
                        ui.painter().rect_filled(
                            egui::Rect::from_min_size(p, egui::vec2(1.0, 1.0)),
                            0.0,
                            GRIP_HI,
                        );
                        ui.painter().rect_filled(
                            egui::Rect::from_min_size(
                                p + egui::vec2(1.0, 1.0),
                                egui::vec2(1.0, 1.0),
                            ),
                            0.0,
                            GRIP_LO,
                        );
                    }
                }
                // Grip dots right
                for gy in (4..20).step_by(3) {
                    for gx in (4..56).step_by(4) {
                        let p = egui::pos2(
                            bar_rect.max.x - 60.0 + gx as f32,
                            bar_rect.min.y + gy as f32,
                        );
                        ui.painter().rect_filled(
                            egui::Rect::from_min_size(p, egui::vec2(1.0, 1.0)),
                            0.0,
                            GRIP_HI,
                        );
                        ui.painter().rect_filled(
                            egui::Rect::from_min_size(
                                p + egui::vec2(1.0, 1.0),
                                egui::vec2(1.0, 1.0),
                            ),
                            0.0,
                            GRIP_LO,
                        );
                    }
                }

                // Centered title text
                ui.painter().text(
                    bar_rect.center(),
                    egui::Align2::CENTER_CENTER,
                    "Vectorize",
                    egui::FontId::monospace(12.0),
                    TEXT_DIM,
                );

                // Close button
                let close_rect = egui::Rect::from_min_size(
                    egui::pos2(bar_rect.max.x - 22.0, bar_rect.min.y + 3.0),
                    egui::vec2(18.0, 16.0),
                );
                let close_resp = ui.allocate_rect(close_rect, egui::Sense::click());
                let close_bg = if close_resp.hovered() {
                    egui::Color32::from_rgb(0xcc, 0x44, 0x44)
                } else {
                    BTN_FACE
                };
                ui.painter().rect_filled(close_rect, 0.0, close_bg);
                ui.painter().text(
                    close_rect.center(),
                    egui::Align2::CENTER_CENTER,
                    "X",
                    egui::FontId::monospace(10.0),
                    BTN_TEXT,
                );
                if close_resp.clicked() {
                    ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                }

                // Minimize button
                let min_rect = egui::Rect::from_min_size(
                    egui::pos2(bar_rect.max.x - 42.0, bar_rect.min.y + 3.0),
                    egui::vec2(18.0, 16.0),
                );
                let min_resp = ui.allocate_rect(min_rect, egui::Sense::click());
                let min_bg = if min_resp.hovered() {
                    BTN_LIGHT
                } else {
                    BTN_FACE
                };
                ui.painter().rect_filled(min_rect, 0.0, min_bg);
                ui.painter().text(
                    min_rect.center(),
                    egui::Align2::CENTER_CENTER,
                    "_",
                    egui::FontId::monospace(10.0),
                    BTN_TEXT,
                );
                if min_resp.clicked() {
                    ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(true));
                }

                // Make title bar draggable
                let drag_rect = egui::Rect::from_min_max(
                    bar_rect.min,
                    egui::pos2(bar_rect.max.x - 44.0, bar_rect.max.y),
                );
                let drag_resp = ui.allocate_rect(drag_rect, egui::Sense::drag());
                if drag_resp.dragged() {
                    ctx.send_viewport_cmd(egui::ViewportCommand::StartDrag);
                }
            });

        // ── Toolbar (26px) ───────────────────────────────────────────────
        egui::TopBottomPanel::top("toolbar")
            .exact_height(self.ui_cfg.toolbar_height)
            .frame(
                egui::Frame::new()
                    .fill(BG_METAL)
                    .inner_margin(egui::Margin::symmetric(6, 2)),
            )
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 3.0;

                    if metal_button(ui, "OPEN", !self.busy) {
                        if let Some(path) = rfd::FileDialog::new()
                            .set_title("Open image")
                            .add_filter("Images", &["png", "jpg", "jpeg", "bmp", "webp"])
                            .pick_file()
                        {
                            self.open_image(path, ctx);
                        }
                    }
                    {
                        let export_enabled = !self.busy && self.svg_string.is_some();
                        if export_enabled {
                            if metal_button(ui, "EXPORT", true) {
                                self.export_svg();
                            }
                        } else {
                            // Candy red when no SVG to export
                            metal_button_colored(ui, "EXPORT", false, egui::Color32::from_rgb(0xE8, 0x50, 0x50));
                        }
                    }

                    ui.add_space(6.0);

                    let dull_purple = RETRO_TEAL;
                    if metal_toggle_colored(ui, "INPUT", self.active_tab == Tab::Input, dull_purple) {
                        self.active_tab = Tab::Input;
                    }
                    if metal_toggle_colored(ui, "OUTPUT", self.active_tab == Tab::Output, dull_purple) {
                        self.active_tab = Tab::Output;
                    }
                });
            });

        // ── Status bar + progress (bottom) ──────────────────────────────
        egui::TopBottomPanel::bottom("statusbar")
            .exact_height(30.0)
            .frame(
                egui::Frame::new()
                    .fill(BG_METAL)
                    .inner_margin(egui::Margin::symmetric(8, 4)),
            )
            .show(ctx, |ui| {
                // Progress bar (7px tall)
                let bar_h = 7.0;
                let bar_rect = egui::Rect::from_min_size(
                    ui.cursor().min,
                    egui::vec2(ui.available_width(), bar_h),
                );
                ui.painter().rect_filled(bar_rect, 0.0, PANEL_BG);
                if self.progress > 0.0 {
                    let fill_w = bar_rect.width() * self.progress;
                    let fill = egui::Rect::from_min_size(
                        bar_rect.min,
                        egui::vec2(fill_w, bar_h),
                    );

                    if self.busy {
                        // Animated rainbow gradient while generating
                        let time = ui.input(|i| i.time) as f32;
                        let stripe_width = 12.0;
                        let speed = 80.0; // pixels per second
                        let offset = time * speed;

                        let colors = [
                            egui::Color32::from_rgb(0xE8, 0x50, 0x50), // red
                            egui::Color32::from_rgb(0xE8, 0xA0, 0x30), // orange
                            egui::Color32::from_rgb(0xE0, 0xD0, 0x30), // yellow
                            egui::Color32::from_rgb(0x50, 0xC8, 0x50), // green
                            egui::Color32::from_rgb(0x40, 0x90, 0xE0), // blue
                            egui::Color32::from_rgb(0x90, 0x60, 0xC0), // purple
                        ];

                        // Use a sub-painter clipped to the fill rect
                        let clipped = ui.painter().with_clip_rect(fill);

                        let cycle = stripe_width * colors.len() as f32;
                        let start_x = fill.min.x - cycle;
                        let end_x = fill.max.x + cycle;
                        let mut x = start_x + (offset % cycle);
                        let mut ci = 0usize;
                        while x < end_x {
                            let stripe = egui::Rect::from_min_max(
                                egui::pos2(x, fill.min.y),
                                egui::pos2(x + stripe_width, fill.max.y),
                            );
                            clipped.rect_filled(stripe, 0.0, colors[ci % colors.len()]);
                            x += stripe_width;
                            ci += 1;
                        }
                    } else {
                        // Static green when done
                        ui.painter()
                            .rect_filled(fill, 0.0, egui::Color32::from_rgb(0x50, 0xC8, 0x50));
                    }
                }
                ui.allocate_space(egui::vec2(0.0, bar_h + 4.0));

                // Status text
                let filename = self
                    .input_path
                    .as_ref()
                    .and_then(|p| p.file_name())
                    .map(|n| n.to_string_lossy().to_string())
                    .unwrap_or_else(|| "No file".into());
                let dims = if self.input_width > 0 {
                    format!("{}x{}", self.input_width, self.input_height)
                } else {
                    "--".into()
                };
                // Compute SVG stats if we have a result
                let svg_stats = if let Some(ref svg) = self.svg_string {
                    let paths = svg.matches("<path").count();
                    let anchors = svg.matches('M').count()
                        + svg.matches('L').count()
                        + svg.matches('C').count()
                        + svg.matches('Q').count();
                    let mut colors: Vec<&str> = Vec::new();
                    let mut pos = 0;
                    while let Some(start) = svg[pos..].find("fill=\"") {
                        let s = pos + start + 6;
                        if let Some(end) = svg[s..].find('"') {
                            let color = &svg[s..s + end];
                            if !colors.contains(&color) {
                                colors.push(color);
                            }
                        }
                        pos = s + 1;
                    }
                    format!("  |  P:{}  A:{}  C:{}", paths, anchors, colors.len())
                } else {
                    String::new()
                };

                let status = format!(
                    "{}  |  {}  |  {}  |  {}{}",
                    filename,
                    dims,
                    self.preset_name(),
                    self.progress_text,
                    svg_stats
                );
                ui.label(
                    egui::RichText::new(status)
                        .monospace()
                        .size(10.0)
                        .color(TEXT_DIM),
                );
            });

        // ── Action button panel (narrow, left of settings) ──────────────
        egui::SidePanel::right("actions")
            .exact_width(100.0)
            .resizable(false)
            .frame(
                egui::Frame::new()
                    .fill(BG_METAL)
                    .inner_margin(egui::Margin::symmetric(6, 6)),
            )
            .show(ctx, |ui| {
                ui.set_min_width(ui.available_width());

                let btn_w = ui.available_width();

                // ── MODE ──────────────────────────────────────────
                section_header(ui, "MODE");
                for (i, label) in MODE_LABELS.iter().enumerate() {
                    if metal_toggle_sized(ui, label, i == self.preset_idx, Some(btn_w)) {
                        self.apply_mode(i);
                    }
                    if i < MODE_LABELS.len() - 1 { ui.add_space(2.0); }
                }

                ui.add_space(6.0);

                // ── ENGINE ────────────────────────────────────────
                section_header(ui, "ENGINE");
                for (i, label) in ENGINE_LABELS.iter().enumerate() {
                    if metal_toggle_sized(ui, label, i == self.engine_idx, Some(btn_w)) {
                        self.engine_idx = i;
                    }
                    if i < ENGINE_LABELS.len() - 1 { ui.add_space(2.0); }
                }

                ui.add_space(6.0);

                // ── ACTIONS ───────────────────────────────────────
                section_header(ui, "ACTIONS");

                // Cancel
                if metal_action_button(ui, "CANCEL", self.busy, btn_w) {
                    self.cancel_vectorize();
                }
                ui.add_space(2.0);

                // Clear
                if metal_action_button(ui, "CLEAR", !self.busy, btn_w) {
                    self.clear_all();
                }
            });

        // ── Right side panel (280px) ─────────────────────────────────────
        egui::SidePanel::right("settings")
            .exact_width(self.ui_cfg.side_panel_width)
            .resizable(false)
            .frame(
                egui::Frame::new()
                    .fill(BG_METAL)
                    .inner_margin(egui::Margin::symmetric(self.ui_cfg.side_panel_margin_x as i8, self.ui_cfg.side_panel_margin_y as i8)),
            )
            .show(ctx, |ui| {
                egui::ScrollArea::vertical().show(ui, |ui| {
                    ui.set_min_width(ui.available_width());

                    // ── QUALITY ───────────────────────────────────────
                    // Apple rainbow: green, yellow, orange, red, purple, blue
                    let c_green  = RETRO_TEAL;
                    let c_yellow = RETRO_YELLOW;
                    let c_orange = RETRO_AMBER;
                    let c_red    = RETRO_ROSE;
                    let c_purple = RETRO_VIOLET;
                    let c_blue   = RETRO_PURPLE;

                    let is_logo = self.preset_idx == 0; // Logo mode
                    let is_sketch = self.preset_idx == 4; // Sketch mode

                    // ── MAIN CONTROLS (always visible) ────────────
                    section_header(ui, "CONTROLS");
                    inset_frame(ui, |ui| {
                        labeled_slider_color_enabled(ui, "Color Detail", &mut self.color_detail, 0.0, 200.0,
                            "The main DETAIL control. Like Illustrator's detail slider.\n\
                             Controls how many color regions AND gradient layers.\n\
                             \n\
                             0 = ~4 colors. Bold posterized look.\n\
                             50 = ~36 colors. Simple illustration.\n\
                             100 = ~130 colors. Good gradients.\n\
                             150 = ~290 colors. Rich photographic detail.\n\
                             200 = ~512 colors. Maximum fidelity.\n\
                             \n\
                             Higher = more detail + larger file size.\n\
                             Lower = fewer shapes + smaller file + graphic look.", c_green, true);
                        labeled_slider_color(ui, "Anchor Density", &mut self.anchor_density, 0.0, 100.0,
                            "How many anchor points along curved paths.\n\
                             More anchors = smoother curves, larger SVG.\n\
                             \n\
                             0 = Minimal. Coarse polygonal shapes.\n\
                             30 = Low. Visible faceting on curves.\n\
                             50 = Balanced. Good for most images.\n\
                             80 = High. Smooth curves, recommended for logos.\n\
                             100 = Maximum. Very smooth, large files.", c_orange);
                        labeled_slider_color_enabled(ui, "Edge Smooth", &mut self.edge_smoothing, 0.0, 3.0,
                            "Smooth edges after color thresholding.\n\
                             Prevents pixel staircases on sharp color boundaries.\n\
                             \n\
                             0.0 = No smoothing. Raw pixel edges.\n\
                             1.0 = Subtle. Light anti-aliasing.\n\
                             1.5 = Default. Clean smooth edges.\n\
                             3.0 = Maximum. Very soft boundaries.", c_purple, true);
                        labeled_slider_color_enabled(ui, "Threshold", &mut self.color_threshold, 0.0, 100.0,
                            "Color threshold for Logo mode.\n\
                             Controls how many colors survive hard thresholding.\n\
                             Lower = fewer colors, cleaner B&W.\n\
                             Higher = more colors, more detail preserved.\n\
                             \n\
                             0 = Pure B&W (2 colors).\n\
                             20 = Default. ~5 colors.\n\
                             50 = ~9 colors.\n\
                             100 = 16 colors (maximum detail).", c_orange, is_logo);
                    });

                    // ── ADVANCED (collapsed by default) ───────────────
                    {
                        let arrow = if self.advanced_expanded { "\u{25BC}" } else { "\u{25B6}" };
                        let header_text = format!("{arrow} ADVANCED");
                        let resp = ui.add(egui::Label::new(
                            egui::RichText::new(&header_text)
                                .size(11.0)
                                .color(egui::Color32::from_rgb(180, 180, 180))
                        ).sense(egui::Sense::click()));
                        if resp.clicked() {
                            self.advanced_expanded = !self.advanced_expanded;
                        }
                    }
                    if self.advanced_expanded {
                    inset_frame(ui, |ui| {
                        // Quality sliders (power-user)
                        labeled_slider_color_enabled(ui, "Path Precision", &mut self.path_precision, 0.0, 200.0,
                            "How tightly paths follow the original pixel edges.\n\
                             \n\
                             0 = Loose/simplified. Fewer segments, smaller file.\n\
                             50 = Balanced. Good general purpose.\n\
                             100 = Pixel-perfect tracing. Largest file size.", c_yellow, true);
                        labeled_slider_color_enabled(ui, "Curve Smooth", &mut self.curve_smoothness, 0.0, 200.0,
                            "How smooth output curves are.\n\
                             \n\
                             0 = Angular/sharp. Preserves all corners.\n\
                             30 = Mild smoothing. Removes pixel stairs.\n\
                             50 = Moderate. Good for organic shapes.\n\
                             80+ = Ultra smooth. Flowing curves, loses fine detail.", c_orange, true);
                        labeled_slider_color_enabled(ui, "Noise Filter", &mut self.noise_filter, 0.0, 200.0,
                            "How aggressively small artifacts are removed.\n\
                             \n\
                             0 = Keep everything, even 1px dots.\n\
                             30 = Light cleanup. Removes dust/noise.\n\
                             70 = Moderate. Removes small features.\n\
                             100 = Aggressive. Only large shapes survive.", c_red, true);
                        labeled_slider_color_enabled(ui, "Gradient Layers", &mut self.gradient_layers, 0.0, 200.0,
                            "How many gradient steps between similar colors.\n\
                             \n\
                             0 = Merge all similar tones. Flat/posterized.\n\
                             50 = Moderate gradient resolution.\n\
                             100 = Fine gradients. Most distinct layers.\n\
                             100+ = Combined with Color Detail for maximum fidelity.", c_purple, true);
                        ui.add_space(4.0);

                        // Tonal detail
                        {
                            let tonal_enabled = !is_sketch;
                            labeled_slider_color_enabled(ui, "Shadows", &mut self.shadow_detail, 0.0, 100.0,
                                "Detail in dark regions (blacks, deep shadows).\n\
                                 \n\
                                 100 = Full detail. All dark tones preserved.\n\
                                 50 = Merge similar darks. Cleaner shadows.\n\
                                 0 = Flatten to 2 shadow tones. Bold/graphic.", c_purple, tonal_enabled);
                            labeled_slider_color_enabled(ui, "Midtones", &mut self.midtone_detail, 0.0, 100.0,
                                "Detail in mid-brightness regions.\n\
                                 \n\
                                 100 = Full detail. All midtones preserved.\n\
                                 50 = Merge similar mids. Simpler gradients.\n\
                                 0 = Flatten to 2 midtone levels.", c_orange, tonal_enabled);
                            labeled_slider_color_enabled(ui, "Highlights", &mut self.highlight_detail, 0.0, 100.0,
                                "Detail in bright regions (whites, highlights).\n\
                                 \n\
                                 100 = Full detail. All bright tones preserved.\n\
                                 50 = Merge similar lights. Cleaner highlights.\n\
                                 0 = Flatten to 2 highlight tones.", c_yellow, tonal_enabled);
                        }
                        ui.add_space(4.0);

                        // Feature toggles & pipeline options
                        metal_checkbox_tip_enabled(ui, &mut self.extract_lines, "Extract Lines",
                            "Separate thin features (text, outlines) into a high-precision\n\
                             binary layer. Best for images with text overlaid on color art.\n\
                             ON = sharper text and outlines, two-pass (slightly slower).\n\
                             OFF = single-pass vectorization.", true);
                        metal_checkbox_tip_enabled(ui, &mut self.detect_shapes, "Detect Shapes",
                            "Replace paths with clean SVG primitives (circles, rectangles)\n\
                             when a shape closely matches a geometric form.\n\
                             Produces smaller files with crisper geometry.\n\
                             (Logo mode always detects shapes)", true);
                        metal_checkbox_tip(ui, &mut self.merge_paths, "Merge Paths",
                            "Combine same-color paths into single elements.\n\
                             ON = fewer elements, smaller file, unified shapes.\n\
                             OFF = individual shapes preserved. Better for logos/editing.");
                        metal_checkbox_tip(ui, &mut self.flatten_background, "Flatten Background",
                            "Render background as a solid rectangle.\n\
                             ON = clean solid background, smaller file.\n\
                             OFF = trace complex background boundaries.\n\
                             (Only affects Native engine)");
                        metal_checkbox_tip(ui, &mut self.skip_transparent, "Skip Transparent",
                            "Skip fully transparent (alpha=0) pixels.\n\
                             ON = transparent areas become empty SVG space.\n\
                             OFF = includes semi-transparent regions.");
                        metal_checkbox_tip(ui, &mut self.snap_curves_to_lines, "Snap To Lines",
                            "Convert nearly-straight bezier curves to true line segments.\n\
                             Produces cleaner geometric output for logos, icons, UI elements.\n\
                             OFF = keep all curves as beziers.");
                        ui.add_space(4.0);

                        // Create: Fills / Strokes
                        ui.horizontal(|ui| {
                            metal_checkbox_tip(ui, &mut self.create_fills, "Fills",
                                "Create filled shapes in the output SVG.");
                            metal_checkbox_tip(ui, &mut self.create_strokes, "Strokes",
                                "Create stroked centerline paths for thin shapes.\n\
                                 Detects elongated regions and converts them to strokes.");
                        });
                        ui.add_space(4.0);

                        // Layer mode
                        if let Some(i) =
                            metal_toggle_row(ui, &LAYER_LABELS, self.layer_idx)
                        {
                            self.layer_idx = i;
                        }
                        ui.add_space(2.0);

                        // Simplify method
                        static SIMPLIFY_LABELS: &[&str] = &["Bezier", "Visvalingam"];
                        if let Some(i) =
                            metal_toggle_row(ui, SIMPLIFY_LABELS, self.simplify_method_idx)
                        {
                            self.simplify_method_idx = i;
                        }
                        ui.add_space(4.0);

                        int_slider_color(ui, "Min Area", &mut self.min_area, 1, 200, "",
                            "Minimum region size in pixels to keep.\n\
                             Smaller regions are discarded as noise.\n\
                             \n\
                             1 = Keep everything (even single pixels).\n\
                             25 = Default. Filters dust/artifacts.\n\
                             100+ = Only large shapes. Removes fine detail.\n\
                             (Only affects Native engine)", c_blue);
                        {
                            let mut v = self.tones_per_hue as u32;
                            int_slider_color(ui, "Tones/Hue", &mut v, 0, 8, "Off",
                                "Limit tonal values per color family.\n\
                                 Like Illustrator's color reduction.\n\
                                 \n\
                                 Off = Unlimited colors (full vtracer output).\n\
                                 2 = Light/dark per hue. Bold, graphic.\n\
                                 3 = Highlight/midtone/shadow. Classic illustration.\n\
                                 5-8 = More subtle gradation per color.", c_green);
                            self.tones_per_hue = v as u8;
                        }
                    });
                    } // end if advanced_expanded

                    // ── LOG ───────────────────────────────────────────
                    section_header(ui, "LOG");
                    let log_h = self.ui_cfg.log_box_height;
                    let full_w = ui.available_width();
                    let inner_w = full_w - 12.0;
                    let log_frame = egui::Frame::new()
                        .fill(PANEL_BG)
                        .inner_margin(egui::Margin::symmetric(6, 4));
                    let resp = log_frame.show(ui, |ui| {
                        ui.set_min_width(inner_w);
                        ui.set_max_width(inner_w);
                        ui.set_min_height(log_h);
                        ui.set_max_height(log_h);
                        egui::ScrollArea::vertical()
                            .max_height(log_h)
                            .stick_to_bottom(true)
                            .show(ui, |ui| {
                                ui.set_max_width(inner_w);
                                for entry in &self.log_entries {
                                    ui.label(
                                        egui::RichText::new(entry)
                                            .monospace()
                                            .size(self.ui_cfg.log_font_size)
                                            .color(TEXT_LCD),
                                    );
                                }
                            });
                    });
                    bevel_sunken(ui, resp.response.rect);

                    // ── VECTORIZE BUTTON ──────────────────────────
                    ui.add_space(6.0);
                    let btn_w = ui.available_width();
                    let can_run = !self.busy && self.input_image.is_some();
                    let green = egui::Color32::from_rgb(0x4A, 0x8A, 0x4A);
                    let btn_bg = if can_run { green } else { BTN_FACE };
                    let (btn_rect, btn_resp) = ui.allocate_exact_size(
                        egui::vec2(btn_w, 24.0), egui::Sense::click());
                    let hovered = btn_resp.hovered() && can_run;
                    let pressed = btn_resp.is_pointer_button_down_on() && can_run;
                    let bg = if pressed {
                        egui::Color32::from_rgb(0x38, 0x6A, 0x38)
                    } else if hovered {
                        egui::Color32::from_rgb(0x5A, 0x9A, 0x5A)
                    } else {
                        btn_bg
                    };
                    ui.painter().rect_filled(btn_rect, 0.0, bg);
                    if pressed { bevel_sunken(ui, btn_rect); } else { bevel_raised(ui, btn_rect); }
                    let font = egui::FontId::monospace(11.0);
                    let fg = if can_run { egui::Color32::WHITE } else { TEXT_DIM };
                    let galley = ui.painter().layout_no_wrap("VECTORIZE".to_string(), font.clone(), fg);
                    let text_pos = egui::pos2(
                        btn_rect.center().x - galley.size().x / 2.0,
                        btn_rect.center().y - galley.size().y / 2.0,
                    );
                    embossed_text(ui, text_pos, "VECTORIZE", font, fg);
                    if btn_resp.clicked() && can_run {
                        self.start_vectorize();
                    }

                }); // end ScrollArea
            });

        // ── Central panel ────────────────────────────────────────────────
        egui::CentralPanel::default()
            .frame(
                egui::Frame::new()
                    .fill(BG_METAL)
                    .inner_margin(egui::Margin::symmetric(6, 0)),
            )
            .show(ctx, |ui| {
                // Content area in dark inset
                let content_frame = egui::Frame::new()
                    .fill(PANEL_BG)
                    .stroke(egui::Stroke::new(1.0, BORDER_IN))
                    .inner_margin(egui::Margin::same(4));
                content_frame.show(ui, |ui| match self.active_tab {
                    Tab::Input => self.render_input_tab(ui),
                    Tab::Output => self.render_output_tab(ui),
                });
            });

    }
}

// ── Tab rendering ────────────────────────────────────────────────────────────

impl VectorizeApp {
    fn render_input_tab(&mut self, ui: &mut egui::Ui) {
        let Some(tex) = self.input_texture.as_ref() else {
            ui.centered_and_justified(|ui| {
                ui.label(
                    egui::RichText::new("Drop or open an image")
                        .monospace()
                        .size(14.0)
                        .color(TEXT_LCD),
                );
            });
            return;
        };

        let tex_id = tex.id();
        let tex_size = tex.size_vec2();
        let available = ui.available_size();
        let base_scale = (available.x / tex_size.x).min(available.y / tex_size.y);
        let scale = base_scale * self.input_zoom;
        let img_size = tex_size * scale;

        let (rect, response) =
            ui.allocate_exact_size(available, egui::Sense::click_and_drag());

        // Scroll wheel zoom — zooms toward cursor position
        if response.hovered() {
            let scroll = ui.input(|i| i.smooth_scroll_delta.y);
            if scroll != 0.0 {
                let old_zoom = self.input_zoom;
                self.input_zoom = (self.input_zoom
                    * if scroll > 0.0 { 1.05 } else { 1.0 / 1.05 })
                .clamp(0.1, 20.0);
                // Adjust offset so the point under the cursor stays fixed
                if let Some(mouse) = ui.input(|i| i.pointer.hover_pos()) {
                    let center = rect.center() + self.input_offset;
                    let mouse_rel = mouse - center;
                    let zoom_ratio = self.input_zoom / old_zoom;
                    self.input_offset += mouse_rel * (1.0 - zoom_ratio);
                }
            }
        }
        // Middle-mouse or right-drag pan
        if response.dragged_by(egui::PointerButton::Middle)
            || response.dragged_by(egui::PointerButton::Secondary)
        {
            self.input_offset += response.drag_delta();
        }
        // Double-click reset
        if response.double_clicked() {
            self.input_zoom = 1.0;
            self.input_offset = egui::Vec2::ZERO;
        }

        let center = rect.center() + self.input_offset;
        let img_rect = egui::Rect::from_center_size(center, img_size);
        ui.painter().image(
            tex_id,
            img_rect,
            egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
            egui::Color32::WHITE,
        );

        // Info overlay
        ui.painter().text(
            egui::pos2(rect.max.x - 4.0, rect.max.y - 4.0),
            egui::Align2::RIGHT_BOTTOM,
            format!(
                "{}x{}  {:.0}%",
                self.input_width,
                self.input_height,
                self.input_zoom * 100.0
            ),
            egui::FontId::monospace(10.0),
            TEXT_LCD,
        );
    }

    fn render_output_tab(&mut self, ui: &mut egui::Ui) {
        let Some(tex) = self.output_texture.as_ref() else {
            ui.centered_and_justified(|ui| {
                ui.label(
                    egui::RichText::new("No output yet.\nRun Vectorize first.")
                        .monospace()
                        .size(14.0)
                        .color(TEXT_LCD),
                );
            });
            return;
        };

        let tex_id = tex.id();
        let tex_size = tex.size_vec2();
        let available = ui.available_size();
        let base_scale = (available.x / tex_size.x).min(available.y / tex_size.y);
        let scale = base_scale * self.output_zoom;
        let img_size = tex_size * scale;

        let (rect, response) =
            ui.allocate_exact_size(available, egui::Sense::click_and_drag());

        // Scroll wheel zoom — zooms toward cursor position
        if response.hovered() {
            let scroll = ui.input(|i| i.smooth_scroll_delta.y);
            if scroll != 0.0 {
                let old_zoom = self.output_zoom;
                self.output_zoom = (self.output_zoom
                    * if scroll > 0.0 { 1.05 } else { 1.0 / 1.05 })
                .clamp(0.1, 20.0);
                // Adjust offset so the point under the cursor stays fixed
                if let Some(mouse) = ui.input(|i| i.pointer.hover_pos()) {
                    let center = rect.center() + self.output_offset;
                    let mouse_rel = mouse - center;
                    let zoom_ratio = self.output_zoom / old_zoom;
                    self.output_offset += mouse_rel * (1.0 - zoom_ratio);
                }
            }
        }
        // Middle-mouse or right-drag pan
        if response.dragged_by(egui::PointerButton::Middle)
            || response.dragged_by(egui::PointerButton::Secondary)
        {
            self.output_offset += response.drag_delta();
        }
        // Double-click reset
        if response.double_clicked() {
            self.output_zoom = 1.0;
            self.output_offset = egui::Vec2::ZERO;
        }

        let center = rect.center() + self.output_offset;
        let img_rect = egui::Rect::from_center_size(center, img_size);
        ui.painter().image(
            tex_id,
            img_rect,
            egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
            egui::Color32::WHITE,
        );

        // Info overlay
        let svg_size = self
            .svg_string
            .as_ref()
            .map(|s| format!("  SVG: {} KB", s.len() / 1024))
            .unwrap_or_default();
        ui.painter().text(
            egui::pos2(rect.max.x - 4.0, rect.max.y - 4.0),
            egui::Align2::RIGHT_BOTTOM,
            format!("{:.0}%{}", self.output_zoom * 100.0, svg_size),
            egui::FontId::monospace(10.0),
            TEXT_LCD,
        );
    }
}

// ── Entry point ──────────────────────────────────────────────────────────────

fn main() -> eframe::Result {
    let cfg = load_ui_config();
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([cfg.window_width, cfg.window_height])
            .with_min_inner_size([cfg.window_min_width, cfg.window_min_height])
            .with_decorations(false)
            .with_title("Vectorize"),
        ..Default::default()
    };

    eframe::run_native(
        "Vectorize",
        options,
        Box::new(|cc| {
            apply_theme(&cc.egui_ctx);
            Ok(Box::new(VectorizeApp::default()))
        }),
    )
}
