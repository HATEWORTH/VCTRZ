use anyhow::{Context, Result};
use clap::Parser;
use std::path::PathBuf;
use tracing_subscriber::EnvFilter;
use vectorize_core::quality::Mode;
use vectorize_core::{Engine, LayerMode, VectorizeConfig};

#[derive(Parser, Debug)]
#[command(name = "vectorize", about = "High-quality raster-to-vector conversion")]
struct Cli {
    /// Input image file (PNG, JPEG, BMP, TIFF, WebP)
    input: PathBuf,

    /// Output SVG file (default: input with .svg extension)
    #[arg(short, long)]
    output: Option<PathBuf>,

    // ── Mode ────────────────────────────────────────────────────
    /// Pipeline mode: logo, illustration, photo, hifi, sketch
    #[arg(short, long, default_value = "illustration", alias = "preset")]
    mode: String,

    // ── Fine-tuning axes (0.0-100.0, override preset values) ────
    /// Color detail (0=posterized, 100=max fidelity)
    #[arg(long)]
    color_detail: Option<f32>,

    /// Path precision (0=simplified, 100=pixel-perfect)
    #[arg(long)]
    path_precision: Option<f32>,

    /// Curve smoothness (0=angular, 100=ultra-smooth)
    #[arg(long)]
    curve_smoothness: Option<f32>,

    /// Noise filter (0=keep everything, 100=aggressive cleanup)
    #[arg(long)]
    noise_filter: Option<f32>,

    /// Gradient layers (0=merge similar tones, 100=every shade)
    #[arg(long)]
    gradient_layers: Option<f32>,

    // ── Tonal sub-controls (0-100) ─────────────────────────────
    /// Shadow detail (0=flatten darks, 100=full detail)
    #[arg(long)]
    shadow_detail: Option<f32>,

    /// Midtone detail (0=flatten mids, 100=full detail)
    #[arg(long)]
    midtone_detail: Option<f32>,

    /// Highlight detail (0=flatten brights, 100=full detail)
    #[arg(long)]
    highlight_detail: Option<f32>,

    // ── Advanced options ─────────────────────────────────────────
    /// Number of colors (0 = auto from preset)
    #[arg(short, long, default_value = "0")]
    colors: u32,

    /// Layer mode: stacked or cutout (default: from mode)
    #[arg(long)]
    layer_mode: Option<String>,

    /// Detect geometric shapes (default: from mode)
    #[arg(long)]
    detect_shapes: Option<bool>,

    /// Extract text/outlines as a separate line layer (default: from mode)
    #[arg(long)]
    extract_lines: Option<bool>,

    /// Disable same-color path merging
    #[arg(long)]
    no_merge: bool,

    /// Don't flatten background to a solid rect
    #[arg(long)]
    no_flatten_bg: bool,

    /// Don't skip transparent pixels
    #[arg(long)]
    no_skip_transparent: bool,

    /// Minimum region area in pixels to trace (1-200)
    #[arg(long)]
    min_area: Option<u32>,

    /// Number of tones per hue group (0=off, 2-8 typical)
    #[arg(long)]
    tones_per_hue: Option<u8>,

    /// Edge smoothing radius for Logo mode (0=jagged, 1.5=default, 3.0=max)
    #[arg(long)]
    edge_smooth: Option<f64>,

    /// Color threshold for Logo mode (0=B&W, 20=default, 100=16 colors)
    #[arg(long)]
    color_threshold: Option<f64>,

    /// Anchor point density on curves (0=coarse, 50=balanced, 100=maximum)
    #[arg(long)]
    anchor_density: Option<f64>,

    /// Simplify method: "bezier" or "visvalingam" (default: from mode)
    #[arg(long)]
    simplify: Option<String>,

    /// Engine: "vtracer", "hybrid", or "native" (default: from mode)
    #[arg(long)]
    engine: Option<String>,

    /// Output config as JSON instead of running
    #[arg(long)]
    dump_config: bool,
}

fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .init();

    let cli = Cli::parse();

    // Parse mode name, then build config from mode recipe
    let mode: Mode = cli
        .mode
        .parse()
        .unwrap_or_else(|_| {
            tracing::warn!("Unknown mode '{}', using 'logo'", cli.mode);
            Mode::Logo
        });

    // Start from the mode's recipe — this sets engine, simplify method,
    // detect_shapes, extract_lines, and all other mode-driven defaults.
    let recipe = mode.recipe();
    let mut config = recipe.to_config();
    config.mode = mode;

    // Apply quality slider overrides (only if explicitly provided)
    if let Some(v) = cli.color_detail { config.quality.color_detail = v; }
    if let Some(v) = cli.path_precision { config.quality.path_precision = v; }
    if let Some(v) = cli.curve_smoothness { config.quality.curve_smoothness = v; }
    if let Some(v) = cli.noise_filter { config.quality.noise_filter = v; }
    if let Some(v) = cli.gradient_layers { config.quality.gradient_layers = v; }
    if let Some(v) = cli.shadow_detail { config.quality.shadow_detail = v; }
    if let Some(v) = cli.midtone_detail { config.quality.midtone_detail = v; }
    if let Some(v) = cli.highlight_detail { config.quality.highlight_detail = v; }

    // Note: corner_threshold, fit_tolerance, simplify_tolerance are recalculated
    // from quality settings at runtime by each pipeline stage, so no need to
    // recompute them here after slider overrides.

    // Apply explicit CLI overrides (these take precedence over mode)
    if cli.colors > 0 { config.color_count = cli.colors; }
    if let Some(v) = cli.min_area { config.min_area = v; }
    if let Some(ref v) = cli.layer_mode {
        config.layer_mode = match v.as_str() {
            "cutout" => LayerMode::Cutout,
            _ => LayerMode::Stacked,
        };
    }
    if let Some(v) = cli.detect_shapes { config.detect_shapes = v; }
    if let Some(v) = cli.extract_lines { config.extract_lines = v; }
    if cli.no_merge { config.merge_paths = false; }
    if cli.no_flatten_bg { config.flatten_background = false; }
    if cli.no_skip_transparent { config.skip_transparent = false; }
    if let Some(v) = cli.tones_per_hue { config.tones_per_hue = v; }
    if let Some(v) = cli.edge_smooth { config.edge_smoothing = v; }
    if let Some(v) = cli.color_threshold { config.color_threshold = v; }
    if let Some(v) = cli.anchor_density { config.anchor_density = v; }
    if let Some(ref v) = cli.simplify {
        config.simplify_method = match v.as_str() {
            "visvalingam" | "vw" => vectorize_core::SimplifyMethod::VisvalingamWhyatt,
            _ => vectorize_core::SimplifyMethod::KurboBezier,
        };
    }
    if let Some(ref v) = cli.engine {
        config.engine = match v.as_str() {
            "hybrid" => Engine::Hybrid,
            "native" => Engine::Native,
            _ => Engine::Vtracer,
        };
    }

    if cli.dump_config {
        println!("{}", serde_json::to_string_pretty(&config)?);
        return Ok(());
    }

    let img = image::ImageReader::open(&cli.input)
        .with_context(|| format!("Failed to open {}", cli.input.display()))?
        .with_guessed_format()
        .with_context(|| "Failed to detect image format")?
        .decode()
        .with_context(|| "Failed to decode image")?;

    tracing::info!(
        "Loaded {} ({}x{}, {:?})",
        cli.input.display(),
        img.width(),
        img.height(),
        img.color()
    );

    let svg = vectorize_core::vectorize(&img, &config)?;

    let output_path = cli.output.unwrap_or_else(|| cli.input.with_extension("svg"));
    std::fs::write(&output_path, &svg)
        .with_context(|| format!("Failed to write {}", output_path.display()))?;

    tracing::info!("Wrote {}", output_path.display());
    Ok(())
}
