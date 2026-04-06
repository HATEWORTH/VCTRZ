use pyo3::prelude::*;
use vectorize_core::quality::{Mode, QualitySettings};

/// Vectorize a raster image file to SVG string.
#[pyfunction]
#[allow(clippy::too_many_arguments)]
#[pyo3(signature = (
    input_path,
    colors = 0,
    mode = "illustration",
    engine = "hybrid",
    detect_shapes = false,
    merge_paths = true,
    tones_per_hue = 0,
    layer_mode = "stacked",
    color_detail = None,
    path_precision = None,
    curve_smoothness = None,
    noise_filter = None,
    gradient_layers = None,
))]
fn vectorize_file(
    py: Python<'_>,
    input_path: &str,
    colors: u32,
    mode: &str,
    engine: &str,
    detect_shapes: bool,
    merge_paths: bool,
    tones_per_hue: u8,
    layer_mode: &str,
    color_detail: Option<f32>,
    path_precision: Option<f32>,
    curve_smoothness: Option<f32>,
    noise_filter: Option<f32>,
    gradient_layers: Option<f32>,
) -> PyResult<String> {
    let config = build_config(
        colors, mode, engine, detect_shapes, merge_paths,
        tones_per_hue, layer_mode,
        color_detail, path_precision, curve_smoothness,
        noise_filter, gradient_layers,
    )?;

    py.allow_threads(|| {
        let img = image::open(input_path)
            .map_err(|e| PyErr::new::<pyo3::exceptions::PyIOError, _>(e.to_string()))?;

        vectorize_core::vectorize(&img, &config)
            .map_err(|e| PyErr::new::<pyo3::exceptions::PyRuntimeError, _>(e.to_string()))
    })
}

/// Vectorize raw image bytes to SVG string.
///
/// Args:
///     data: Raw image file bytes (PNG, JPEG, etc. — not raw pixels)
///     colors: Number of colors (0 = auto-detect)
///     mode: Pipeline mode ("logo", "illustration", "photo", "hifi", "sketch")
///     engine: Tracing engine ("vtracer", "hybrid", "native")
///
/// Returns:
///     SVG string
#[pyfunction]
#[allow(clippy::too_many_arguments)]
#[pyo3(signature = (
    data,
    colors = 0,
    mode = "illustration",
    engine = "hybrid",
    detect_shapes = false,
    merge_paths = true,
    tones_per_hue = 0,
    layer_mode = "stacked",
    color_detail = None,
    path_precision = None,
    curve_smoothness = None,
    noise_filter = None,
    gradient_layers = None,
))]
fn vectorize_bytes(
    py: Python<'_>,
    data: &[u8],
    colors: u32,
    mode: &str,
    engine: &str,
    detect_shapes: bool,
    merge_paths: bool,
    tones_per_hue: u8,
    layer_mode: &str,
    color_detail: Option<f32>,
    path_precision: Option<f32>,
    curve_smoothness: Option<f32>,
    noise_filter: Option<f32>,
    gradient_layers: Option<f32>,
) -> PyResult<String> {
    let config = build_config(
        colors, mode, engine, detect_shapes, merge_paths,
        tones_per_hue, layer_mode,
        color_detail, path_precision, curve_smoothness,
        noise_filter, gradient_layers,
    )?;

    py.allow_threads(|| {
        let img = image::load_from_memory(data)
            .map_err(|e| PyErr::new::<pyo3::exceptions::PyValueError, _>(e.to_string()))?;

        vectorize_core::vectorize(&img, &config)
            .map_err(|e| PyErr::new::<pyo3::exceptions::PyRuntimeError, _>(e.to_string()))
    })
}

/// Build a VectorizeConfig from Python arguments.
#[allow(clippy::too_many_arguments)]
fn build_config(
    colors: u32,
    mode: &str,
    engine: &str,
    detect_shapes: bool,
    merge_paths: bool,
    tones_per_hue: u8,
    layer_mode: &str,
    color_detail: Option<f32>,
    path_precision: Option<f32>,
    curve_smoothness: Option<f32>,
    noise_filter: Option<f32>,
    gradient_layers: Option<f32>,
) -> PyResult<vectorize_core::VectorizeConfig> {
    let mode_enum: Mode = mode.parse().map_err(|e: String| {
        PyErr::new::<pyo3::exceptions::PyValueError, _>(e)
    })?;

    let mut quality = QualitySettings::from_mode(mode_enum);
    if let Some(v) = color_detail { quality = quality.with_color_detail(v); }
    if let Some(v) = path_precision { quality = quality.with_path_precision(v); }
    if let Some(v) = curve_smoothness { quality = quality.with_curve_smoothness(v); }
    if let Some(v) = noise_filter { quality = quality.with_noise_filter(v); }
    if let Some(v) = gradient_layers { quality = quality.with_gradient_layers(v); }

    let engine_enum = match engine.to_lowercase().as_str() {
        "vtracer" => vectorize_core::Engine::Vtracer,
        "hybrid" => vectorize_core::Engine::Hybrid,
        "native" => vectorize_core::Engine::Native,
        other => {
            return Err(PyErr::new::<pyo3::exceptions::PyValueError, _>(
                format!("unknown engine: '{other}'. Options: vtracer, hybrid, native"),
            ));
        }
    };

    Ok(vectorize_core::VectorizeConfig {
        color_count: colors,
        engine: engine_enum,
        detect_shapes,
        merge_paths,
        tones_per_hue,
        layer_mode: match layer_mode {
            "cutout" => vectorize_core::LayerMode::Cutout,
            _ => vectorize_core::LayerMode::Stacked,
        },
        quality,
        mode: mode_enum,
        ..Default::default()
    })
}

#[pymodule]
fn vectorize(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_function(wrap_pyfunction!(vectorize_file, m)?)?;
    m.add_function(wrap_pyfunction!(vectorize_bytes, m)?)?;
    Ok(())
}
