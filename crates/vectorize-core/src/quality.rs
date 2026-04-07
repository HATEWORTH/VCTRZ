//! Quality modes and fine-tuning controls.
//!
//! A **Mode** is a pipeline recipe — it controls which algorithms run, which
//! pipeline stages are active, and sets sensible defaults for all quality axes.
//! Users can still tweak individual sliders, but the mode defines the baseline
//! behavior and algorithm selection.

use serde::{Deserialize, Serialize};
use crate::{Engine, LayerMode, SimplifyMethod};

/// User-facing quality controls. Each axis is 0.0-100.0 (fractional values allowed).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QualitySettings {
    /// How many distinct colors to preserve.
    /// 0 = posterized (~8 colors), 100 = maximum fidelity (~256 per channel).
    pub color_detail: f32,

    /// How tightly paths follow the original pixel edges.
    /// 0 = loose/simplified, 100 = pixel-perfect tracing.
    pub path_precision: f32,

    /// How smooth output curves are.
    /// 0 = angular/polygon, 100 = ultra-smooth flowing curves.
    pub curve_smoothness: f32,

    /// How aggressively small artifacts are filtered.
    /// 0 = keep everything (even 1px dots), 100 = remove all small features.
    pub noise_filter: f32,

    /// How many gradient steps between similar colors.
    /// 0 = merge all similar tones, 100 = separate every shade.
    pub gradient_layers: f32,

    // ── Tonal sub-controls (0-100) ─────────────────────────────────
    // Fine-tune color detail per luminance band. 100 = full detail (no change),
    // 0 = aggressively merge tones in that band.

    /// Shadow detail — controls tonal variation in dark regions (Oklab L < 0.33).
    pub shadow_detail: f32,
    /// Midtone detail — controls tonal variation in mid-brightness regions.
    pub midtone_detail: f32,
    /// Highlight detail — controls tonal variation in bright regions (Oklab L > 0.66).
    pub highlight_detail: f32,
}

/// Pipeline mode — controls which algorithms and stages run.
/// A Mode is a complete pipeline recipe that determines algorithm selection,
/// feature flags, parameter ranges, and quality slider defaults.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Mode {
    /// Flat graphics, text, icons. Few colors, hard edges, geometric shapes.
    /// Prioritizes: shape detection, geometric simplification, clean edges.
    Logo,
    /// Cartoon, clipart, flat illustrations. Organic curves, moderate detail.
    /// Prioritizes: Visvalingam simplification, stroke detection, smooth curves.
    Illustration,
    /// Photographs and realistic images. Many colors, smooth gradients.
    /// Prioritizes: gradient preservation, tonal bands, maximum color fidelity.
    Photo,
    /// Maximum quality. Preserves every detail regardless of file size.
    /// Prioritizes: lowest tolerances, most colors, least simplification.
    HighFidelity,
    /// Pencil/ink sketches. Line extraction, minimal fills, high contrast.
    /// Prioritizes: line extraction, few colors, aggressive noise filtering.
    Sketch,
}

/// A pipeline recipe — the full set of defaults and algorithm choices for a mode.
/// Each mode produces one of these, which is then used to build a `VectorizeConfig`.
#[derive(Debug, Clone)]
pub struct ModeRecipe {
    // Quality slider defaults
    pub color_detail: f32,
    pub path_precision: f32,
    pub curve_smoothness: f32,
    pub noise_filter: f32,
    pub gradient_layers: f32,

    // Algorithm selection
    pub preferred_engine: Engine,
    pub simplify_method: SimplifyMethod,

    // Feature flags
    pub detect_shapes: bool,
    pub extract_lines: bool,
    pub merge_paths: bool,
    pub flatten_background: bool,
    pub skip_transparent: bool,

    // Pipeline parameters
    pub layer_mode: LayerMode,
    pub tones_per_hue: u8,
    pub color_count_range: (u32, u32),
    pub min_area: u32,
    /// Edge smoothing radius for Logo mode (0.0 = jagged, 1.0-3.0 = smooth).
    pub edge_smoothing: f64,
    /// Color threshold for Logo mode (0-100). 0 = 2 colors (B&W), 100 = 16 colors.
    pub color_threshold: f64,
    /// Anchor point density (0-100). More = smoother curves, larger files.
    pub anchor_density: f64,
}

impl Mode {
    /// Get the pipeline recipe for this mode.
    pub fn recipe(&self) -> ModeRecipe {
        match self {
            Mode::Logo => ModeRecipe {
                color_detail: 40.0,
                path_precision: 100.0,
                curve_smoothness: 25.0,
                noise_filter: 30.0,
                gradient_layers: 15.0,
                preferred_engine: Engine::Hybrid,
                simplify_method: SimplifyMethod::KurboBezier,
                detect_shapes: true,
                extract_lines: false,
                merge_paths: true,
                flatten_background: true,
                skip_transparent: true,
                layer_mode: LayerMode::Stacked,

                tones_per_hue: 0,
                color_count_range: (2, 32),
                min_area: 25,
                edge_smoothing: 1.5,
                color_threshold: 20.0,
                anchor_density: 80.0,
            },
            Mode::Illustration => ModeRecipe {
                color_detail: 50.0,
                path_precision: 60.0,
                curve_smoothness: 35.0,
                noise_filter: 50.0,
                gradient_layers: 30.0,
                preferred_engine: Engine::Hybrid,
                simplify_method: SimplifyMethod::VisvalingamWhyatt,
                detect_shapes: false,
                extract_lines: false,
                merge_paths: true,
                flatten_background: true,
                skip_transparent: true,
                layer_mode: LayerMode::Stacked,

                tones_per_hue: 0,
                color_count_range: (16, 64),
                min_area: 25,
                edge_smoothing: 1.0,
                color_threshold: 50.0,
                anchor_density: 50.0,
            },
            Mode::Photo => ModeRecipe {
                color_detail: 80.0,
                path_precision: 55.0,
                curve_smoothness: 60.0,
                noise_filter: 60.0,
                gradient_layers: 60.0,
                preferred_engine: Engine::Vtracer,
                simplify_method: SimplifyMethod::KurboBezier,
                detect_shapes: false,
                extract_lines: false,
                merge_paths: true,
                flatten_background: true,
                skip_transparent: true,
                layer_mode: LayerMode::Stacked,

                tones_per_hue: 0,
                color_count_range: (64, 512),
                min_area: 10,
                edge_smoothing: 0.8,
                color_threshold: 50.0,
                anchor_density: 60.0,
            },
            Mode::HighFidelity => ModeRecipe {
                color_detail: 150.0,
                path_precision: 150.0,
                curve_smoothness: 15.0,
                noise_filter: 5.0,
                gradient_layers: 90.0,
                preferred_engine: Engine::Hybrid,
                simplify_method: SimplifyMethod::KurboBezier,
                detect_shapes: true,
                extract_lines: true,
                merge_paths: true,
                flatten_background: true,
                skip_transparent: true,
                layer_mode: LayerMode::Stacked,

                tones_per_hue: 0,
                color_count_range: (128, 512),
                min_area: 2,
                edge_smoothing: 0.3,
                color_threshold: 50.0,
                anchor_density: 90.0,
            },
            Mode::Sketch => ModeRecipe {
                color_detail: 25.0,
                path_precision: 70.0,
                curve_smoothness: 15.0,
                noise_filter: 60.0,
                gradient_layers: 10.0,
                preferred_engine: Engine::Hybrid,
                simplify_method: SimplifyMethod::KurboBezier,
                detect_shapes: false,
                extract_lines: true,
                merge_paths: true,
                flatten_background: true,
                skip_transparent: true,
                layer_mode: LayerMode::Stacked,

                tones_per_hue: 0,
                color_count_range: (2, 16),
                min_area: 10,
                edge_smoothing: 0.5,
                color_threshold: 50.0,
                anchor_density: 40.0,
            },
        }
    }

}

impl ModeRecipe {
    /// Build a complete VectorizeConfig from this recipe.
    /// Quality sliders use the mode defaults; everything else is set by the recipe.
    pub fn to_config(&self) -> crate::VectorizeConfig {
        let quality = QualitySettings {
            color_detail: self.color_detail,
            path_precision: self.path_precision,
            curve_smoothness: self.curve_smoothness,
            noise_filter: self.noise_filter,
            gradient_layers: self.gradient_layers,
            shadow_detail: 100.0,
            midtone_detail: 100.0,
            highlight_detail: 100.0,
        };

        crate::VectorizeConfig {
            color_count: 0, // auto-detect within range
            min_area: self.min_area,
            corner_threshold: quality.native_corner_threshold(),
            fit_tolerance: quality.native_fit_tolerance(),
            simplify_tolerance: quality.native_simplify_tolerance(),
            simplify_method: self.simplify_method,
            layer_mode: self.layer_mode,
            detect_shapes: self.detect_shapes,
            skip_transparent: self.skip_transparent,
            flatten_background: self.flatten_background,
            merge_paths: self.merge_paths,
            tones_per_hue: self.tones_per_hue,
            engine: self.preferred_engine,
            quality,
            extract_lines: self.extract_lines,
            edge_smoothing: self.edge_smoothing,
            color_threshold: self.color_threshold,
            anchor_density: self.anchor_density,
            snap_curves_to_lines: false,
            create_fills: true,
            create_strokes: false,
            max_dimension: 0,
            mode: Mode::Illustration, // will be set by caller
        }
    }
}

impl Default for Mode {
    fn default() -> Self {
        Mode::Logo
    }
}

impl std::fmt::Display for Mode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Mode::Logo => write!(f, "Logo"),
            Mode::Illustration => write!(f, "Illustration"),
            Mode::Photo => write!(f, "Photo"),
            Mode::HighFidelity => write!(f, "HiFi"),
            Mode::Sketch => write!(f, "Sketch"),
        }
    }
}

impl std::str::FromStr for Mode {
    type Err = String;

    fn from_str(s: &str) -> std::result::Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "logo" => Ok(Self::Logo),
            "illustration" | "illust" => Ok(Self::Illustration),
            "photo" => Ok(Self::Photo),
            "high-fidelity" | "highfidelity" | "hifi" | "max" => Ok(Self::HighFidelity),
            "sketch" => Ok(Self::Sketch),
            _ => Err(format!("unknown mode: '{s}'. Options: logo, illustration, photo, hifi, sketch")),
        }
    }
}

/// Build a VectorizeConfig from a mode with optional user overrides on the quality sliders.
/// This is the recommended way to create configs — mode sets the recipe, user tweaks the sliders.
pub fn build_config(mode: Mode, quality_overrides: Option<&QualitySettings>) -> crate::VectorizeConfig {
    let recipe = mode.recipe();
    let mut config = recipe.to_config();
    config.mode = mode;

    // Apply user slider overrides if provided
    if let Some(overrides) = quality_overrides {
        config.quality.color_detail = overrides.color_detail;
        config.quality.path_precision = overrides.path_precision;
        config.quality.curve_smoothness = overrides.curve_smoothness;
        config.quality.noise_filter = overrides.noise_filter;
        config.quality.gradient_layers = overrides.gradient_layers;
        config.quality.shadow_detail = overrides.shadow_detail;
        config.quality.midtone_detail = overrides.midtone_detail;
        config.quality.highlight_detail = overrides.highlight_detail;

        // Recalculate derived values from updated quality
        config.corner_threshold = config.quality.native_corner_threshold();
        config.fit_tolerance = config.quality.native_fit_tolerance();
        config.simplify_tolerance = config.quality.native_simplify_tolerance();
    }

    config
}

impl Default for QualitySettings {
    fn default() -> Self {
        Self::from_mode(Mode::Illustration)
    }
}

impl QualitySettings {
    /// Create settings from a mode with default axis values.
    #[must_use]
    pub fn from_mode(mode: Mode) -> Self {
        //                       color  path  smooth  noise  gradient
        let (color, path, smooth, noise, gradient) = match mode {
            Mode::Logo =>         (40.0, 100.0, 25.0, 30.0, 15.0),
            Mode::Illustration => (50.0,  60.0, 35.0, 50.0, 30.0),
            Mode::Photo =>        (80.0,  55.0, 60.0, 60.0, 60.0),
            Mode::HighFidelity => (95.0,  85.0, 20.0, 20.0, 85.0),
            Mode::Sketch =>       (25.0,  70.0, 15.0, 60.0, 10.0),
        };

        Self {
            color_detail: color,
            path_precision: path,
            curve_smoothness: smooth,
            noise_filter: noise,
            gradient_layers: gradient,
            shadow_detail: 100.0,
            midtone_detail: 100.0,
            highlight_detail: 100.0,
        }
    }

    /// Override a single axis. Returns self for chaining.
    #[must_use]
    pub fn with_color_detail(mut self, value: f32) -> Self {
        self.color_detail = value.clamp(0.0, 200.0);
        self
    }

    #[must_use]
    pub fn with_path_precision(mut self, value: f32) -> Self {
        self.path_precision = value.clamp(0.0, 200.0);
        self
    }

    #[must_use]
    pub fn with_curve_smoothness(mut self, value: f32) -> Self {
        self.curve_smoothness = value.clamp(0.0, 200.0);
        self
    }

    #[must_use]
    pub fn with_noise_filter(mut self, value: f32) -> Self {
        self.noise_filter = value.clamp(0.0, 200.0);
        self
    }

    #[must_use]
    pub fn with_gradient_layers(mut self, value: f32) -> Self {
        self.gradient_layers = value.clamp(0.0, 200.0);
        self
    }

    #[must_use]
    pub fn with_shadow_detail(mut self, value: f32) -> Self {
        self.shadow_detail = value.clamp(0.0, 100.0);
        self
    }

    #[must_use]
    pub fn with_midtone_detail(mut self, value: f32) -> Self {
        self.midtone_detail = value.clamp(0.0, 100.0);
        self
    }

    #[must_use]
    pub fn with_highlight_detail(mut self, value: f32) -> Self {
        self.highlight_detail = value.clamp(0.0, 100.0);
        self
    }

    // ── Internal mapping to vtracer parameters ──────────────────────

    /// VTracer color_precision: bits per channel (1-8).
    /// Color Detail 0→4 bits, 50→6 bits, 80+→8 bits (max).
    pub(crate) fn vtracer_color_precision(&self) -> i32 {
        let clamped = self.color_detail.min(100.0);
        if clamped >= 60.0 {
            8
        } else {
            lerp_i32(clamped * 100.0 / 60.0, 4, 8)
        }
    }

    /// Maps Color Detail 0-200 to a color count.
    /// This is the primary "detail" control — like Illustrator's detail slider.
    ///
    /// 0 = 4 colors, 50 = ~12, 100 = ~128, 150 = ~350, 200 = 512.
    pub(crate) fn auto_color_count_hint(&self) -> u32 {
        let t = self.color_detail as f64 / 200.0; // normalized to 0-1 over 0-200 range
        let count = 4.0 + 508.0 * t * t; // quadratic: 4→512
        (count.round() as u32).clamp(4, 512)
    }

    /// VTracer filter_speckle: side length of minimum feature to keep.
    /// VTracer squares this internally to get area.
    /// Low values preserve fine detail (hatching, thin lines, text).
    pub(crate) fn vtracer_filter_speckle(&self) -> usize {
        // 0-20→0 (keep everything), 50→2, 100→4, 200→8
        if self.noise_filter < 20.0 {
            0
        } else {
            lerp_usize((self.noise_filter - 20.0) * 100.0 / 180.0, 0, 8)
        }
    }

    /// VTracer layer_difference: color difference between gradient layers.
    /// Lower = more gradient steps = more layers = finer detail.
    /// Both gradient_layers AND color_detail influence this — whichever
    /// demands more detail wins (lower layer_difference).
    pub(crate) fn vtracer_layer_difference(&self) -> i32 {
        // Gradient layers: 0→128, 50→16, 100→1
        let gl_clamped = self.gradient_layers.min(100.0);
        let from_gl = if gl_clamped >= 80.0 {
            1
        } else {
            lerp_i32(100.0 - gl_clamped, 1, 128)
        };
        // Color detail: 0→128, 50→32, 100→4, 150→2, 200→1
        let cd_clamped = self.color_detail.min(200.0);
        let from_cd = if cd_clamped <= 20.0 {
            128
        } else if cd_clamped >= 150.0 {
            1
        } else {
            lerp_i32((150.0 - cd_clamped) * 100.0 / 130.0, 1, 128)
        };
        // Take the minimum (more detail wins).
        from_gl.min(from_cd).max(1)
    }

    /// VTracer corner_threshold: minimum angle (degrees) to be a corner.
    /// Lower = sharper edge detection. Higher = smoother.
    /// 0→5° (extremely sharp), 100→90°, 200→150° (very smooth).
    pub(crate) fn vtracer_corner_threshold(&self) -> i32 {
        let cs = self.curve_smoothness.min(200.0);
        lerp_i32(cs / 2.0, 5, 150)
    }

    /// VTracer splice_threshold: minimum angle (degrees) for splicing.
    pub(crate) fn vtracer_splice_threshold(&self) -> i32 {
        let cs = self.curve_smoothness.min(200.0);
        lerp_i32(cs / 2.0, 5, 120)
    }

    /// VTracer length_threshold: max segment length for path fitting.
    /// Lower = more segments = smoother curves.
    pub(crate) fn vtracer_length_threshold(&self) -> f64 {
        // 0→5.0, 100→0.3, 200→0.1
        let inv = (200.0 - self.path_precision).max(0.0);
        lerp_f64(inv / 2.0, 0.1, 5.0)
    }

    /// VTracer max_iterations: curve fitting iterations.
    /// More = higher fidelity but slower.
    pub(crate) fn vtracer_max_iterations(&self) -> usize {
        // 0→2, 100→15, 200→30 (capped — higher causes vtracer hangs)
        let pp = self.path_precision.min(200.0);
        lerp_usize(pp / 2.0, 2, 30)
    }

    /// VTracer path simplify mode.
    /// Spline is the default — it produces actual bezier curves.
    /// Polygon is only for extreme sharpness (slider at 0).
    pub(crate) fn vtracer_path_mode(&self) -> visioncortex::PathSimplifyMode {
        if self.curve_smoothness < 1.0 {
            visioncortex::PathSimplifyMode::Polygon
        } else {
            visioncortex::PathSimplifyMode::Spline
        }
    }

    /// Hybrid engine: tolerance for kurbo re-fitting of vtracer paths.
    /// Path Precision controls the base (higher precision = tighter tolerance).
    /// Curve Smoothness adds tolerance for flowing curves.
    /// At max precision (200) and zero smoothness: tolerance = 0.3 (very tight edges).
    pub(crate) fn hybrid_refit_tolerance(&self) -> f64 {
        // Base: path_precision 0→5.0, 100→1.0, 200→0.3
        let pp = self.path_precision.min(200.0);
        let base = lerp_f64((200.0 - pp) / 2.0, 0.3, 5.0);
        // Smoothness boost: only adds tolerance when smoothness > 0
        let cs = self.curve_smoothness.min(200.0);
        let smooth_boost = lerp_f64(cs / 2.0, 0.0, 3.0);
        base + smooth_boost
    }

    /// Hybrid engine: kurbo simplify angle threshold.
    /// Controls how aggressively corners are preserved vs smoothed.
    /// Lower = more corners preserved = sharper edges.
    pub(crate) fn hybrid_angle_thresh(&self) -> f64 {
        // curve_smoothness 0→1.0 (very sharp edges), 100→0.01, 200→0.0001
        let cs = self.curve_smoothness.min(200.0);
        lerp_f64((200.0 - cs) / 2.0, 0.0001, 1.0)
    }

    /// Hybrid engine: selective Chaikin smoothing iterations.
    /// Only smooths small-angle corners (pixel stairs), preserves real features.
    /// 0 = no smoothing, 1-4 = progressive smoothing.
    pub(crate) fn hybrid_smooth_iterations(&self) -> u32 {
        // curve_smoothness 0→0, 15→1, 35→2, 60→3, 80→4
        if self.curve_smoothness < 15.0 {
            0
        } else if self.curve_smoothness < 35.0 {
            1
        } else if self.curve_smoothness < 60.0 {
            2
        } else if self.curve_smoothness < 80.0 {
            3
        } else {
            4
        }
    }

    /// Whether tonal band preprocessing is needed.
    /// Returns true if any band has detail < 100 (i.e., user wants reduction).
    pub fn needs_tonal_preprocessing(&self) -> bool {
        self.shadow_detail < 99.0 || self.midtone_detail < 99.0 || self.highlight_detail < 99.0
    }

    /// Compression strength per tonal band (0.0 = no compression, 1.0 = full collapse).
    /// Maps the user's "detail" slider (100 = full detail, 0 = flatten) to a
    /// compression factor that smoothly merges tones within each band.
    /// Returns (shadow_strength, midtone_strength, highlight_strength).
    pub fn tonal_compression(&self) -> (f32, f32, f32) {
        fn detail_to_strength(detail: f32) -> f32 {
            // 100 → 0.0 (no compression), 0 → 1.0 (full collapse)
            let t = 1.0 - (detail / 100.0).clamp(0.0, 1.0);
            // Quadratic for perceptual feel — small slider changes at the
            // top end (90-100) do very little, big changes at the bottom.
            t * t
        }
        (
            detail_to_strength(self.shadow_detail),
            detail_to_strength(self.midtone_detail),
            detail_to_strength(self.highlight_detail),
        )
    }

    /// Adaptive curve refinement options for VTracer output.
    /// Returns `Some` when curve_smoothness is in a range where polygon/near-polygon
    /// output would benefit from selective bezier fitting on curved sections.
    /// Returns `None` when smoothness is high enough that VTracer's spline mode
    /// already produces smooth curves.
    pub(crate) fn vtracer_refine_options(&self) -> Option<crate::refine::RefineOptions> {
        // Only refine when curve_smoothness < 50 — this is the "crisp" zone
        // where VTracer produces polygon-like output with faceted curves.
        // Above 50, VTracer's spline mode handles smoothing well enough.
        if self.curve_smoothness >= 50.0 {
            return None;
        }

        // Map curve_smoothness 0-50 to refinement aggressiveness:
        // - At 0: very selective (high corner threshold, tight fit)
        // - At 50: more aggressive smoothing (lower corner threshold, looser fit)
        let t = self.curve_smoothness / 50.0; // 0.0 to 1.0

        Some(crate::refine::RefineOptions {
            // Corner threshold: 25° at cs=0 → 45° at cs=50
            // Lower = more corners preserved = crisper
            corner_threshold_deg: 25.0 + 20.0 * t as f64,
            // Straight threshold: 2° at cs=0 → 5° at cs=50
            straight_threshold_deg: 2.0 + 3.0 * t as f64,
            // Min curve run: 4 at cs=0 → 3 at cs=50
            min_curve_run: if self.curve_smoothness < 25.0 { 4 } else { 3 },
            // Fit tolerance: 0.5 at cs=0 → 1.5 at cs=50
            // Tighter = more faithful to original polygon points
            fit_tolerance: 0.5 + 1.0 * t as f64,
        })
    }

    /// Post-process merge: per-channel color tolerance for merging same-color paths.
    /// Higher color_detail = tighter tolerance (fewer merges).
    pub(crate) fn merge_color_tolerance(&self) -> u8 {
        // color_detail 0→20, 100→2, 200→0
        // Inverted — higher detail = stricter matching
        let inv = (200.0 - self.color_detail).max(0.0);
        lerp_i32(inv / 2.0, 0, 20).max(0) as u8
    }

    /// Shape detection: max deviation (px) to accept a geometric match.
    pub(crate) fn shape_detection_tolerance(&self) -> f64 {
        // 0→4.0, 50→2.0, 100→0.5
        lerp_f64((200.0 - self.path_precision).max(0.0) / 2.0, 0.5, 4.0)
    }

    /// VTracer path_precision: decimal places in SVG output.
    /// Higher = more coordinate precision = tighter paths.
    pub(crate) fn vtracer_path_precision(&self) -> Option<u32> {
        if self.path_precision > 120.0 {
            Some(4)
        } else if self.path_precision > 60.0 {
            Some(3)
        } else if self.path_precision > 30.0 {
            Some(2)
        } else {
            Some(1)
        }
    }

    // ── Native pipeline parameter mappings ───────────────────────────

    /// Native pipeline: Bezier fitting tolerance (pixels).
    /// Lower = more accurate, higher = fewer segments.
    pub(crate) fn native_fit_tolerance(&self) -> f64 {
        lerp_f64((200.0 - self.path_precision).max(0.0) / 2.0, 0.5, 3.0)
    }

    /// Native pipeline: path simplification tolerance.
    /// Applied after curve fitting to reduce segment count.
    /// Must be > fit_tolerance to have any effect.
    pub(crate) fn native_simplify_tolerance(&self) -> f64 {
        // Always 1.5x the fit tolerance so simplification runs.
        // curve_smoothness 0→fit*1.1 (barely simplify), 100→fit*3.0 (aggressive)
        let fit = self.native_fit_tolerance();
        let factor = lerp_f64(self.curve_smoothness, 1.1, 3.0);
        fit * factor
    }

    /// Native pipeline: corner detection threshold (degrees).
    pub(crate) fn native_corner_threshold(&self) -> f64 {
        // Same mapping as vtracer corner threshold but as f64.
        lerp_f64(self.curve_smoothness, 10.0, 120.0)
    }

    /// Native pipeline: minimum region area to trace (pixels).
    pub(crate) fn native_min_area(&self) -> u32 {
        // noise_filter 0→1 (keep everything), 100→100 (aggressive)
        lerp_f64(self.noise_filter, 1.0, 100.0).round() as u32
    }
}

/// Linear interpolation for i32: value 0→min, 100→max.
fn lerp_i32(value: f32, min: i32, max: i32) -> i32 {
    let t = (value / 100.0) as f64;
    (f64::from(min) + t * f64::from(max - min)).round() as i32
}

/// Linear interpolation for usize.
fn lerp_usize(value: f32, min: usize, max: usize) -> usize {
    let t = (value / 100.0) as f64;
    (min as f64 + t * (max as f64 - min as f64)).round() as usize
}

/// Linear interpolation for f64.
fn lerp_f64(value: f32, min: f64, max: f64) -> f64 {
    let t = (value / 100.0) as f64;
    min + t * (max - min)
}

impl std::fmt::Display for QualitySettings {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "[color={:.1} path={:.1} smooth={:.1} noise={:.1} gradient={:.1}]",
            self.color_detail,
            self.path_precision,
            self.curve_smoothness,
            self.noise_filter,
            self.gradient_layers,
        )
    }
}


#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_mode_defaults() {
        let logo = QualitySettings::from_mode(Mode::Logo);
        assert!((logo.color_detail - 40.0).abs() < 0.01);
        assert!((logo.path_precision - 100.0).abs() < 0.01);
        assert!((logo.curve_smoothness - 25.0).abs() < 0.01);
        assert!((logo.noise_filter - 30.0).abs() < 0.01);
        assert!((logo.gradient_layers - 15.0).abs() < 0.01);

        let hifi = QualitySettings::from_mode(Mode::HighFidelity);
        assert!((hifi.color_detail - 95.0).abs() < 0.01);
        assert!((hifi.path_precision - 85.0).abs() < 0.01);
    }

    #[test]
    fn test_lerp_extremes() {
        assert_eq!(lerp_i32(0.0, 3, 8), 3);
        assert_eq!(lerp_i32(100.0, 3, 8), 8);
        assert_eq!(lerp_i32(50.0, 0, 100), 50);
    }

    #[test]
    fn test_vtracer_color_precision_range() {
        let low = QualitySettings::from_mode(Mode::Logo);
        let high = QualitySettings::from_mode(Mode::HighFidelity);
        assert!(low.vtracer_color_precision() < high.vtracer_color_precision());
        assert!(low.vtracer_color_precision() >= 3);
        assert!(high.vtracer_color_precision() <= 8);
    }

    #[test]
    fn test_chaining() {
        let settings = QualitySettings::from_mode(Mode::Photo)
            .with_color_detail(100.0)
            .with_noise_filter(0.0);
        assert!((settings.color_detail - 100.0).abs() < 0.01);
        assert!((settings.noise_filter - 0.0).abs() < 0.01);
        assert!((settings.curve_smoothness - 60.0).abs() < 0.01);
    }

    #[test]
    fn test_clamp_over_200() {
        let s = QualitySettings::from_mode(Mode::Illustration).with_color_detail(255.0);
        assert!((s.color_detail - 200.0).abs() < 0.01);
    }

    #[test]
    fn test_fractional_values() {
        let s = QualitySettings::from_mode(Mode::Illustration)
            .with_curve_smoothness(15.3)
            .with_noise_filter(2.7);
        assert!((s.curve_smoothness - 15.3).abs() < 0.01);
        assert!((s.noise_filter - 2.7).abs() < 0.01);
    }

    #[test]
    fn test_mode_parse() {
        assert_eq!("logo".parse::<Mode>().unwrap(), Mode::Logo);
        assert_eq!("hifi".parse::<Mode>().unwrap(), Mode::HighFidelity);
        assert_eq!("photo".parse::<Mode>().unwrap(), Mode::Photo);
        assert!("nonsense".parse::<Mode>().is_err());
    }

    #[test]
    fn test_display() {
        let s = QualitySettings::from_mode(Mode::Logo);
        let display = format!("{s}");
        assert!(display.contains("color=40.0"));
    }
}
