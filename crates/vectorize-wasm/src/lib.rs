use wasm_bindgen::prelude::*;

/// Initialize panic hook so Rust panics show readable messages in the browser console.
#[wasm_bindgen(start)]
pub fn init() {
    console_error_panic_hook::set_once();
}

/// Full config passed from JS — mirrors all controls the web UI exposes.
#[derive(serde::Deserialize)]
struct JsConfig {
    mode: Option<String>,
    engine: Option<String>,
    quality: Option<JsQuality>,

    // Pipeline parameters
    anchor_density: Option<f64>,
    edge_smoothing: Option<f64>,
    color_threshold: Option<f64>,
    min_area: Option<u32>,
    tones_per_hue: Option<u8>,

    // Feature toggles
    detect_shapes: Option<bool>,
    extract_lines: Option<bool>,
    merge_paths: Option<bool>,
    flatten_background: Option<bool>,
    skip_transparent: Option<bool>,
    snap_curves_to_lines: Option<bool>,
    create_fills: Option<bool>,
    create_strokes: Option<bool>,

    // Algorithm selection
    layer_mode: Option<String>,
    simplify_method: Option<String>,
}

#[derive(serde::Deserialize)]
struct JsQuality {
    color_detail: Option<f32>,
    path_precision: Option<f32>,
    curve_smoothness: Option<f32>,
    noise_filter: Option<f32>,
    gradient_layers: Option<f32>,
    shadow_detail: Option<f32>,
    midtone_detail: Option<f32>,
    highlight_detail: Option<f32>,
}

/// Maximum pixels allowed in WASM (single-threaded, limited memory).
const MAX_PIXELS: u32 = 16_000_000;

/// Vectorize image bytes to SVG string.
///
/// Pass the raw file bytes (PNG, JPEG, etc.) — NOT decoded pixel data.
/// Returns an SVG string.
#[wasm_bindgen]
pub fn vectorize(data: &[u8], config_js: JsValue) -> Result<String, JsError> {
    // Parse the JS config
    let js: Option<JsConfig> = if config_js.is_undefined() || config_js.is_null() {
        None
    } else {
        Some(serde_wasm_bindgen::from_value(config_js).map_err(|e| JsError::new(&e.to_string()))?)
    };

    // Build a full config from mode recipe (just like CLI/GUI does)
    let mode = js
        .as_ref()
        .and_then(|c| c.mode.as_deref())
        .map(|m| m.parse().unwrap_or(vectorize_core::quality::Mode::Logo))
        .unwrap_or(vectorize_core::quality::Mode::Logo);

    let recipe = mode.recipe();
    let mut config = recipe.to_config();
    config.mode = mode;

    if let Some(ref js) = js {
        // Engine
        if let Some(ref e) = js.engine {
            config.engine = match e.as_str() {
                "Hybrid" => vectorize_core::Engine::Hybrid,
                "Native" => vectorize_core::Engine::Native,
                _ => vectorize_core::Engine::Vtracer,
            };
        }

        // Quality sliders
        if let Some(ref q) = js.quality {
            if let Some(v) = q.color_detail { config.quality.color_detail = v; }
            if let Some(v) = q.path_precision { config.quality.path_precision = v; }
            if let Some(v) = q.curve_smoothness { config.quality.curve_smoothness = v; }
            if let Some(v) = q.noise_filter { config.quality.noise_filter = v; }
            if let Some(v) = q.gradient_layers { config.quality.gradient_layers = v; }
            if let Some(v) = q.shadow_detail { config.quality.shadow_detail = v; }
            if let Some(v) = q.midtone_detail { config.quality.midtone_detail = v; }
            if let Some(v) = q.highlight_detail { config.quality.highlight_detail = v; }
        }

        // Pipeline parameters
        if let Some(v) = js.anchor_density { config.anchor_density = v; }
        if let Some(v) = js.edge_smoothing { config.edge_smoothing = v; }
        if let Some(v) = js.color_threshold { config.color_threshold = v; }
        if let Some(v) = js.min_area { config.min_area = v; }
        if let Some(v) = js.tones_per_hue { config.tones_per_hue = v; }

        // Feature toggles
        if let Some(v) = js.detect_shapes { config.detect_shapes = v; }
        if let Some(v) = js.extract_lines { config.extract_lines = v; }
        if let Some(v) = js.merge_paths { config.merge_paths = v; }
        if let Some(v) = js.flatten_background { config.flatten_background = v; }
        if let Some(v) = js.skip_transparent { config.skip_transparent = v; }
        if let Some(v) = js.snap_curves_to_lines { config.snap_curves_to_lines = v; }
        if let Some(v) = js.create_fills { config.create_fills = v; }
        if let Some(v) = js.create_strokes { config.create_strokes = v; }

        // Algorithm selection
        if let Some(ref v) = js.layer_mode {
            config.layer_mode = match v.as_str() {
                "Cutout" => vectorize_core::LayerMode::Cutout,
                _ => vectorize_core::LayerMode::Stacked,
            };
        }
        if let Some(ref v) = js.simplify_method {
            config.simplify_method = match v.as_str() {
                "visvalingam" => vectorize_core::SimplifyMethod::VisvalingamWhyatt,
                _ => vectorize_core::SimplifyMethod::KurboBezier,
            };
        }
    }

    // Decode image (no filesystem in WASM)
    let img =
        image::load_from_memory(data).map_err(|e| JsError::new(&format!("decode: {e}")))?;

    // Check image size — WASM is single-threaded with limited memory
    let pixels = img.width() * img.height();
    if pixels > MAX_PIXELS {
        return Err(JsError::new(&format!(
            "Image too large for browser: {}x{} ({:.1}M pixels). Max is {:.0}M pixels. \
             Please resize the image before uploading.",
            img.width(), img.height(),
            pixels as f64 / 1_000_000.0,
            MAX_PIXELS as f64 / 1_000_000.0
        )));
    }

    // Wrap in catch_unwind to turn panics into error messages
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        vectorize_core::vectorize(&img, &config)
    }));

    match result {
        Ok(Ok(svg)) => Ok(svg),
        Ok(Err(e)) => Err(JsError::new(&e.to_string())),
        Err(_) => Err(JsError::new(
            "Vectorization crashed. Try a smaller image, fewer colors (lower Color Detail), \
             or a different mode."
        )),
    }
}
