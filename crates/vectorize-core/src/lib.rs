//! vectorize-core: High-quality raster-to-vector conversion engine.
//!
//! Two backends:
//! - **vtracer** (default) — mature engine with hierarchical color clustering
//! - **native** — our own pipeline (preprocess → segment → trace → fit → output)
//!
//! The vtracer backend produces better results today. The native pipeline
//! exists for cases where we add capabilities vtracer lacks (ML edge detection,
//! geometric primitive fitting, Oklab color science, GPU acceleration).

pub mod backend;
pub mod fit;
pub mod line_layer;
pub mod merge;
pub mod palette;
pub mod par;
pub mod quality;
pub mod shapes;
pub mod optimize;
pub mod output;
pub mod preprocess;
pub mod refine;
pub mod segment;
pub mod simplify;
pub mod trace;

use image::DynamicImage;
use thiserror::Error;

#[derive(Error, Debug)]
pub enum VectorizeError {
    #[error("image loading failed: {0}")]
    ImageLoad(#[from] image::ImageError),

    #[error("image is empty (0x0)")]
    EmptyImage,

    #[error("unsupported color type: {0:?}")]
    UnsupportedColorType(image::ColorType),

    #[error("tracing failed: {0}")]
    TracingFailed(String),

    #[error("curve fitting failed: {0}")]
    FittingFailed(String),

    #[error("segmentation failed: {0}")]
    SegmentationFailed(String),
}

pub type Result<T> = std::result::Result<T, VectorizeError>;

// ── Shared types that flow between pipeline stages ──

/// Color as RGBA bytes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub struct Color {
    pub r: u8,
    pub g: u8,
    pub b: u8,
    pub a: u8,
}

impl Color {
    #[must_use]
    pub fn rgb(r: u8, g: u8, b: u8) -> Self {
        Self { r, g, b, a: 255 }
    }

    #[must_use]
    pub fn rgba(r: u8, g: u8, b: u8, a: u8) -> Self {
        Self { r, g, b, a }
    }

    /// Format as SVG color string.
    #[must_use]
    pub fn to_svg_color(&self) -> String {
        if self.a == 255 {
            format!("#{:02x}{:02x}{:02x}", self.r, self.g, self.b)
        } else {
            format!(
                "rgba({},{},{},{:.3})",
                self.r,
                self.g,
                self.b,
                f64::from(self.a) / 255.0
            )
        }
    }
}

/// Output of preprocessing: cleaned image + transparency mask.
pub struct PreparedImage {
    /// The RGBA image with transparent pixels premultiplied against white.
    pub image: image::RgbaImage,
    /// Per-pixel opacity mask. `true` = opaque (include in vectorization).
    /// `None` if the source had no transparency.
    pub opaque_mask: Option<Vec<bool>>,
    pub width: u32,
    pub height: u32,
}

/// Output of segmentation: a label map + color palette.
pub struct SegmentedImage {
    /// Each pixel's cluster label (`0..palette.len()`).
    /// Transparent pixels get label `u32::MAX`.
    pub labels: Vec<u32>,
    pub width: u32,
    pub height: u32,
    /// The quantized color palette (does NOT include transparent).
    pub palette: Vec<Color>,
    /// Index of the background color in palette, if detected.
    pub background_label: Option<u32>,
}

/// A traced contour — ordered boundary points with associated color.
pub struct TracedContour {
    pub points: Vec<kurbo::Point>,
    pub color: Color,
    pub is_hole: bool,
}

/// A fitted vector path ready for SVG output.
#[derive(Clone)]
pub struct VectorPath {
    pub path: kurbo::BezPath,
    pub color: Color,
    pub is_hole: bool,
}

// ── Configuration ──

/// Top-level vectorization configuration.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct VectorizeConfig {
    /// Target number of colors for quantization. 0 = auto-detect.
    pub color_count: u32,

    /// Minimum region area in pixels to trace. Regions smaller than this are dropped.
    /// Equivalent to Potrace's "turdsize". Filters noise and JPEG artifacts.
    pub min_area: u32,

    /// Corner detection threshold in degrees (0-180).
    /// Lower = more corners detected. Higher = smoother curves.
    pub corner_threshold: f64,

    /// Bezier fitting tolerance in pixels.
    /// Lower = more accurate (more segments). Higher = smoother (fewer segments).
    pub fit_tolerance: f64,

    /// Path simplification tolerance.
    /// Applied after curve fitting to reduce segment count.
    pub simplify_tolerance: f64,

    /// Path simplification algorithm.
    pub simplify_method: SimplifyMethod,

    /// Layer compositing mode.
    pub layer_mode: LayerMode,

    /// Whether to detect and fit geometric primitives (circles, rectangles).
    pub detect_shapes: bool,

    /// Skip fully transparent pixels (alpha=0) during vectorization.
    /// Prevents checkerboard artifacts from PNG transparency.
    pub skip_transparent: bool,

    /// Render the background as a full-canvas rectangle instead of
    /// tracing its complex boundary. Reduces path count and SVG size.
    pub flatten_background: bool,

    /// Merge paths of the same (or nearly same) fill color into single
    /// multi-subpath elements, and detect stroke-like shapes.
    pub merge_paths: bool,

    /// Max tonal values per hue group (0 = unlimited).
    /// E.g., 3 = highlight/midtone/shadow per color family.
    /// Produces clean illustration-like output.
    pub tones_per_hue: u8,

    /// Which tracing engine to use.
    pub engine: Engine,

    /// Quality controls — preset + fine-tuning axes (each 0-100).
    pub quality: quality::QualitySettings,

    /// Extract thin high-contrast features (text, outlines) as a separate
    /// binary layer traced with maximum precision. Composited on top of the
    /// color layer in the final SVG.
    pub extract_lines: bool,

    /// Edge smoothing radius for Logo mode (0.0 = no smoothing, 1.0-3.0 typical).
    /// Controls how much the thresholded edges are smoothed to prevent pixel staircases.
    /// Only applies in Logo mode.
    pub edge_smoothing: f64,

    /// Color threshold for Logo mode (0.0-100.0).
    /// Controls how many colors survive thresholding.
    /// 0 = 2 colors (pure B&W), 50 = ~6 colors, 100 = ~16 colors (more detail).
    /// Only applies in Logo mode.
    pub color_threshold: f64,

    /// Anchor point density (0.0-100.0). Controls how many points are placed
    /// along curved paths. More anchors = smoother curves but larger files.
    /// 0 = minimal (coarse polygons), 50 = balanced, 100 = maximum density.
    pub anchor_density: f64,

    /// Snap nearly-straight bezier curves to true line segments.
    /// Produces cleaner geometric output at the cost of losing subtle curves.
    pub snap_curves_to_lines: bool,

    /// Create filled shapes in the output SVG.
    pub create_fills: bool,

    /// Create stroked paths in the output SVG (centerline extraction).
    pub create_strokes: bool,

    /// Pipeline mode — controls algorithm selection, feature flags, and
    /// parameter ranges. Unlike quality sliders (which tune intensity),
    /// the mode determines *which* techniques are used at each stage.
    pub mode: quality::Mode,
}

/// Tracing engine selection.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum Engine {
    /// VTracer backend — hierarchical color clustering, best quality.
    Vtracer,
    /// Hybrid — vtracer for clustering + kurbo for curve refitting + shape detection.
    Hybrid,
    /// Native pipeline — our own k-means + contour tracing pipeline.
    Native,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum LayerMode {
    /// Shapes stacked back-to-front (painter's algorithm). Compact output.
    Stacked,
    /// Non-overlapping shapes. Required for cutting machines, laser cutters.
    Cutout,
}

/// Path simplification algorithm.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
pub enum SimplifyMethod {
    /// Kurbo's Bezier-aware simplifier (default). Best for curves.
    #[default]
    KurboBezier,
    /// Visvalingam-Whyatt area-based simplification. Best for organic shapes.
    VisvalingamWhyatt,
}

impl Default for VectorizeConfig {
    fn default() -> Self {
        Self {
            color_count: 0,
            min_area: 25,
            corner_threshold: 60.0,
            fit_tolerance: 1.0,
            simplify_tolerance: 2.0,
            simplify_method: SimplifyMethod::default(),
            layer_mode: LayerMode::Stacked,
            detect_shapes: false,
            skip_transparent: true,
            flatten_background: true,
            merge_paths: true,
            tones_per_hue: 0,
            engine: Engine::Vtracer,
            quality: quality::QualitySettings::default(),
            extract_lines: false,
            edge_smoothing: 1.5,
            color_threshold: 20.0,
            anchor_density: 50.0,
            snap_curves_to_lines: false,
            create_fills: true,
            create_strokes: false,
            mode: quality::Mode::default(),
        }
    }
}

/// Shared progress state for non-blocking progress reporting.
/// The vectorizer writes to this; the UI reads from it on its own schedule.
pub struct ProgressState {
    pub current: std::sync::atomic::AtomicUsize,
    pub total: std::sync::atomic::AtomicUsize,
    /// 0=clustering, 1=refitting, 2=merging, 3=building SVG
    pub stage: std::sync::atomic::AtomicUsize,
    /// Set to true by the UI to request cancellation.
    pub cancelled: std::sync::atomic::AtomicBool,
}

impl ProgressState {
    pub fn new() -> Self {
        Self {
            current: std::sync::atomic::AtomicUsize::new(0),
            total: std::sync::atomic::AtomicUsize::new(0),
            stage: std::sync::atomic::AtomicUsize::new(0),
            cancelled: std::sync::atomic::AtomicBool::new(false),
        }
    }

    pub fn stage_name(&self) -> &'static str {
        match self.stage.load(std::sync::atomic::Ordering::Relaxed) {
            0 => "Color clustering...",
            1 => "Refitting paths",
            2 => "Merging paths...",
            3 => "Building SVG...",
            _ => "Processing...",
        }
    }
}

impl Default for ProgressState {
    fn default() -> Self {
        Self::new()
    }
}

/// Vectorize a raster image to SVG string.
///
/// This is the main entry point. Dispatches to the selected engine.
/// Default engine is VTracer (best quality). Use `Engine::Native` for
/// our own pipeline when you need features vtracer doesn't support.
pub fn vectorize(image: &DynamicImage, config: &VectorizeConfig) -> Result<String> {
    let width = image.width();
    let height = image.height();

    if width == 0 || height == 0 {
        return Err(VectorizeError::EmptyImage);
    }

    tracing::info!(
        "Vectorizing {}x{} image with {} colors (engine: {:?})",
        width,
        height,
        if config.color_count == 0 {
            "auto".to_string()
        } else {
            config.color_count.to_string()
        },
        config.engine,
    );

    // Line-layer extraction: two-pass vectorization for crisp text/outlines.
    // Color layer = full original image traced normally (preserves all structure).
    // Line layer = thin text/outline features traced with max precision, overlaid on top.
    if config.extract_lines {
        let rgba = image.to_rgba8();
        if let Some(extraction) = line_layer::extract_line_mask(&rgba) {
            let t0 = crate::par::instant_now();

            // Build the binary line image for separate tracing
            let line_img = DynamicImage::ImageRgba8(
                line_layer::build_line_image(&rgba, &extraction),
            );

            // Vectorize full original image with normal settings (color layer)
            let mut color_config = config.clone();
            color_config.extract_lines = false; // Prevent recursion
            let color_svg = vectorize_engine(image, &color_config)?;

            // Vectorize line layer with max-precision settings
            let line_config = VectorizeConfig {
                color_count: 2,
                min_area: 4,
                merge_paths: true,
                flatten_background: true,
                extract_lines: false,
                engine: config.engine,
                quality: quality::QualitySettings::from_mode(quality::Mode::Logo)
                    .with_path_precision(100.0)
                    .with_curve_smoothness(10.0)
                    .with_noise_filter(20.0),
                ..config.clone()
            };
            let line_svg = vectorize_engine(&line_img, &line_config)?;

            let merged = line_layer::merge_svg_layers(
                &color_svg,
                &line_svg,
                extraction.bg_color,
            );

            tracing::info!("Line extraction + dual-pass completed in {:?}", t0.elapsed());
            return Ok(post_process_svg(&merged, config));
        }
        // No lines detected — fall through to normal vectorization
    }

    let svg = vectorize_engine(image, config)?;

    // Post-processing: apply snap-to-lines and fills/strokes filtering
    let svg = post_process_svg(&svg, config);

    Ok(svg)
}

/// Apply global post-processing to the SVG output.
/// - Force-close all subpaths (permanent fix for stray diagonal lines)
/// - Snap curves to lines (if enabled)
/// - Filter fills/strokes (if toggled)
fn post_process_svg(svg: &str, config: &VectorizeConfig) -> String {
    // Force-close every subpath to prevent stray diagonal lines.
    let mut result = sanitize_all_paths(svg);

    // Snap curves to lines: replace nearly-straight C/Q commands with L
    if config.snap_curves_to_lines {
        result = snap_svg_curves_to_lines(&result);
    }

    // Filter output types based on create_fills / create_strokes
    if !config.create_fills || !config.create_strokes {
        result = filter_svg_output_types(&result, config.create_fills, config.create_strokes);
    }

    result
}

/// Sanitize every <path> element in an SVG string:
/// Force-close all subpaths (prevents stray diagonal lines).
fn sanitize_all_paths(svg: &str) -> String {
    let mut result = String::with_capacity(svg.len());
    let mut pos = 0;

    while pos < svg.len() {
        if let Some(path_start) = svg[pos..].find("<path") {
            let abs_start = pos + path_start;
            result.push_str(&svg[pos..abs_start]);

            if let Some(end_offset) = svg[abs_start..].find("/>") {
                let abs_end = abs_start + end_offset + 2;
                let elem = &svg[abs_start..abs_end];

                // Re-serialize the d attribute with forced closures
                let mut fixed_elem = elem.to_string();
                if let Some(d_attr_start) = elem.find("d=\"") {
                    let d_start = d_attr_start + 3;
                    if let Some(d_end) = elem[d_start..].find('"') {
                        let d_attr = &elem[d_start..d_start + d_end];
                        if let Ok(bez) = kurbo::BezPath::from_svg(d_attr) {
                            let fixed_d = output::bezpath_to_svg_d(&bez);
                            fixed_elem = format!(
                                "{}{}{}",
                                &elem[..d_start],
                                fixed_d,
                                &elem[d_start + d_end..]
                            );
                        }
                    }
                }

                result.push_str(&fixed_elem);
                pos = abs_end;
            } else {
                result.push_str(&svg[abs_start..abs_start + 5]);
                pos = abs_start + 5;
            }
        } else {
            result.push_str(&svg[pos..]);
            break;
        }
    }

    result
}

/// Replace nearly-straight bezier curve commands in SVG path d attributes
/// with line commands. Operates on the raw SVG string.
fn snap_svg_curves_to_lines(svg: &str) -> String {
    // Parse each <path> element, convert its d attribute
    let mut result = String::with_capacity(svg.len());
    let mut pos = 0;

    while pos < svg.len() {
        if let Some(path_start) = svg[pos..].find("<path") {
            let abs_start = pos + path_start;
            // Copy everything before this path
            result.push_str(&svg[pos..abs_start]);

            if let Some(end_offset) = svg[abs_start..].find("/>") {
                let abs_end = abs_start + end_offset + 2;
                let elem = &svg[abs_start..abs_end];

                // Extract and process the d attribute
                if let (Some(d_start), Some(fill)) = (
                    elem.find("d=\"").map(|p| p + 3),
                    elem.find("fill=\""),
                ) {
                    if let Some(d_end) = elem[d_start..].find('"') {
                        let d_attr = &elem[d_start..d_start + d_end];
                        if let Ok(mut bez) = kurbo::BezPath::from_svg(d_attr) {
                            bez = backend::logo::snap_curves_to_lines(&bez, 0.5);
                            let new_d = output::bezpath_to_svg_d(&bez);
                            // Rebuild the element with the new d
                            result.push_str(&elem[..d_start]);
                            result.push_str(&new_d);
                            result.push_str(&elem[d_start + d_end..]);
                        } else {
                            result.push_str(elem);
                        }
                    } else {
                        result.push_str(elem);
                    }
                } else {
                    result.push_str(elem);
                }
                pos = abs_end;
            } else {
                result.push_str(&svg[abs_start..abs_start + 5]);
                pos = abs_start + 5;
            }
        } else {
            result.push_str(&svg[pos..]);
            break;
        }
    }

    result
}

/// Filter SVG elements based on fills/strokes toggle.
/// If create_fills=false, remove filled paths (keep only stroked ones).
/// If create_strokes=false, remove stroked-only paths (keep filled ones).
fn filter_svg_output_types(svg: &str, create_fills: bool, create_strokes: bool) -> String {
    if create_fills && create_strokes {
        return svg.to_string();
    }

    let mut result = String::with_capacity(svg.len());
    let mut pos = 0;

    while pos < svg.len() {
        if let Some(path_start) = svg[pos..].find("<path") {
            let abs_start = pos + path_start;
            result.push_str(&svg[pos..abs_start]);

            if let Some(end_offset) = svg[abs_start..].find("/>") {
                let abs_end = abs_start + end_offset + 2;
                let elem = &svg[abs_start..abs_end];

                let has_fill = elem.contains("fill=\"") && !elem.contains("fill=\"none\"");
                let is_stroke_only = elem.contains("fill=\"none\"") && elem.contains("stroke=\"");

                let keep = if is_stroke_only {
                    create_strokes
                } else if has_fill {
                    create_fills
                } else {
                    true // rect, circle, etc. — always keep
                };

                if keep {
                    result.push_str(elem);
                }
                pos = abs_end;
            } else {
                result.push_str(&svg[abs_start..abs_start + 5]);
                pos = abs_start + 5;
            }
        } else {
            result.push_str(&svg[pos..]);
            break;
        }
    }

    result
}



/// Dispatch to the selected engine. Internal helper for vectorize().
/// Logo mode gets its own pipeline regardless of engine setting.
fn vectorize_engine(image: &DynamicImage, config: &VectorizeConfig) -> Result<String> {
    tracing::info!("Mode: {:?} | Engine: {:?}", config.mode, config.engine);

    // Logo mode: always use the logo-specific pipeline
    if config.mode == quality::Mode::Logo {
        return backend::logo::vectorize_logo(image, config);
    }

    match config.engine {
        Engine::Vtracer => {
            let t0 = crate::par::instant_now();
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                backend::vtracer_backend::vectorize_with_vtracer(image, config)
            }));
            tracing::info!("VTracer engine completed in {:?}", t0.elapsed());
            match result {
                Ok(r) => r,
                Err(_) => Err(VectorizeError::TracingFailed(
                    "vtracer panicked — try reducing color detail or image size".into(),
                )),
            }
        }
        Engine::Hybrid => {
            let t0 = crate::par::instant_now();
            let result = backend::hybrid::vectorize_hybrid(image, config);
            tracing::info!("Hybrid engine completed in {:?}", t0.elapsed());
            result
        }
        Engine::Native => vectorize_native(image, config),
    }
}

/// Vectorize with shared progress state for non-blocking UI feedback.
///
/// The vectorizer writes progress to `state`; the UI reads it independently.
pub fn vectorize_with_progress(
    image: &DynamicImage,
    config: &VectorizeConfig,
    state: &ProgressState,
) -> Result<String> {
    let width = image.width();
    let height = image.height();

    if width == 0 || height == 0 {
        return Err(VectorizeError::EmptyImage);
    }

    tracing::info!(
        "Vectorizing {}x{} image with {} colors (engine: {:?})",
        width,
        height,
        if config.color_count == 0 {
            "auto".to_string()
        } else {
            config.color_count.to_string()
        },
        config.engine,
    );

    // Logo mode: always use the logo-specific pipeline
    if config.mode == quality::Mode::Logo {
        let svg = backend::logo::vectorize_logo_with_progress(image, config, state)?;
        return Ok(post_process_svg(&svg, config));
    }

    let svg = match config.engine {
        Engine::Vtracer => {
            let t0 = crate::par::instant_now();
            let result = backend::vtracer_backend::vectorize_with_vtracer(image, config);
            tracing::info!("VTracer engine completed in {:?}", t0.elapsed());
            result
        }
        Engine::Hybrid => {
            let t0 = crate::par::instant_now();
            let result = backend::hybrid::vectorize_hybrid_with_progress(image, config, state);
            tracing::info!("Hybrid engine completed in {:?}", t0.elapsed());
            result
        }
        Engine::Native => vectorize_native(image, config),
    }?;

    Ok(post_process_svg(&svg, config))
}

/// Native vectorization pipeline.
fn vectorize_native(image: &DynamicImage, config: &VectorizeConfig) -> Result<String> {
    let width = image.width();
    let height = image.height();

    let t0 = crate::par::instant_now();

    // Stage 1: Preprocess (alpha handling, background detection)
    let prepared = preprocess::prepare(image, config);
    tracing::info!("Stage 1 (preprocess): {:?}", t0.elapsed());

    // Stage 2: Segment (color quantization + region labeling)
    let t1 = crate::par::instant_now();
    let segmented = segment::quantize_and_segment(&prepared, config)?;
    tracing::info!(
        "Stage 2 (segment): {:?} — {} colors",
        t1.elapsed(),
        segmented.palette.len()
    );

    // Stage 3: Trace contours from segmented regions
    let t2 = crate::par::instant_now();
    let contours = trace::extract_contours(&segmented, config);
    tracing::info!(
        "Stage 3 (trace): {:?} — {} contours",
        t2.elapsed(),
        contours.len()
    );

    if contours.is_empty() {
        tracing::warn!("No contours found — producing empty SVG");
        return Ok(output::empty_svg(width, height));
    }

    // Stage 4: Fit Bezier curves to contours
    let t3 = crate::par::instant_now();
    let paths = fit::fit_curves(&contours, config);
    tracing::info!("Stage 4 (fit): {:?} — {} paths", t3.elapsed(), paths.len());

    // Stage 5: Simplify paths
    let t4 = crate::par::instant_now();
    let paths = simplify::simplify_paths(&paths, config);
    tracing::info!(
        "Stage 5 (simplify): {:?} — {} paths",
        t4.elapsed(),
        paths.len()
    );

    // Stage 6: Optimize layer ordering
    let t5 = crate::par::instant_now();
    let paths = optimize::optimize_layers(&paths, config);
    tracing::info!(
        "Stage 6 (optimize): {:?} — {} paths",
        t5.elapsed(),
        paths.len()
    );

    // Resolve background color from segmentation
    let background_color = if config.flatten_background {
        segmented
            .background_label
            .and_then(|lbl| segmented.palette.get(lbl as usize).copied())
    } else {
        None
    };

    // Stage 7: Output SVG
    let t6 = crate::par::instant_now();
    let svg = output::to_svg(&paths, width, height, background_color);
    tracing::info!("Stage 7 (output): {:?} — {} bytes", t6.elapsed(), svg.len());
    tracing::info!("Total: {:?} — {} paths", t0.elapsed(), paths.len());

    Ok(svg)
}
