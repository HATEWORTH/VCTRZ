use wasm_bindgen::prelude::*;

/// Initialize panic hook so Rust panics show readable messages in the browser console.
#[wasm_bindgen(start)]
pub fn init() {
    console_error_panic_hook::set_once();
}

/// Partial config passed from JS — only the fields the web UI exposes.
#[derive(serde::Deserialize)]
struct JsConfig {
    mode: Option<String>,
    quality: Option<JsQuality>,
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

/// Vectorize image bytes to SVG string.
///
/// Pass the raw file bytes (PNG, JPEG, etc.) — NOT decoded pixel data.
/// Returns an SVG string.
#[wasm_bindgen]
pub fn vectorize(data: &[u8], config_js: JsValue) -> Result<String, JsError> {
    // Parse the partial JS config
    let js_config: Option<JsConfig> = if config_js.is_undefined() || config_js.is_null() {
        None
    } else {
        Some(serde_wasm_bindgen::from_value(config_js).map_err(|e| JsError::new(&e.to_string()))?)
    };

    // Build a full config from mode recipe (just like CLI does)
    let mode = js_config
        .as_ref()
        .and_then(|c| c.mode.as_deref())
        .map(|m| m.parse().unwrap_or(vectorize_core::quality::Mode::Logo))
        .unwrap_or(vectorize_core::quality::Mode::Logo);

    let recipe = mode.recipe();
    let mut config = recipe.to_config();
    config.mode = mode;

    // Apply quality overrides from JS sliders
    if let Some(ref js) = js_config {
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
    }

    // NOTE: image::open() does not work in WASM (no filesystem).
    // Always use load_from_memory with byte slices passed from JS.
    let img =
        image::load_from_memory(data).map_err(|e| JsError::new(&format!("decode: {e}")))?;

    vectorize_core::vectorize(&img, &config).map_err(|e| JsError::new(&e.to_string()))
}
