use wasm_bindgen::prelude::*;

/// Initialize panic hook so Rust panics show readable messages in the browser console.
#[wasm_bindgen(start)]
pub fn init() {
    console_error_panic_hook::set_once();
}

/// Vectorize image bytes to SVG string.
///
/// Pass the raw file bytes (PNG, JPEG, etc.) — NOT decoded pixel data.
/// Returns an SVG string.
#[wasm_bindgen]
pub fn vectorize(data: &[u8], config_js: JsValue) -> Result<String, JsError> {
    let config: vectorize_core::VectorizeConfig = if config_js.is_undefined() || config_js.is_null()
    {
        vectorize_core::VectorizeConfig::default()
    } else {
        serde_wasm_bindgen::from_value(config_js).map_err(|e| JsError::new(&e.to_string()))?
    };

    // NOTE: image::open() does not work in WASM (no filesystem).
    // Always use load_from_memory with byte slices passed from JS.
    let img =
        image::load_from_memory(data).map_err(|e| JsError::new(&format!("decode: {e}")))?;

    vectorize_core::vectorize(&img, &config).map_err(|e| JsError::new(&e.to_string()))
}
