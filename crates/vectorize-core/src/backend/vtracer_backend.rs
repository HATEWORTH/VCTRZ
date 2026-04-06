//! VTracer backend — wraps the vtracer crate for high-quality color vectorization.
//!
//! Our preprocessing runs FIRST (alpha handling, premultiply against white),
//! then the cleaned image is passed to vtracer's clustering engine.
//! Post-processing adds a background rect to fix octagonal corner artifacts.

use image::DynamicImage;

use crate::quality::QualitySettings;
use crate::{Result, VectorizeConfig, VectorizeError};

/// Map QualitySettings to vtracer's Config.
fn build_vtracer_config(quality: &QualitySettings, config: &VectorizeConfig) -> vtracer::Config {
    let mut vconfig = vtracer::Config {
        color_mode: vtracer::ColorMode::Color,
        hierarchical: match config.layer_mode {
            crate::LayerMode::Stacked => vtracer::Hierarchical::Stacked,
            crate::LayerMode::Cutout => vtracer::Hierarchical::Cutout,
        },
        color_precision: quality.vtracer_color_precision(),
        filter_speckle: quality.vtracer_filter_speckle(),
        layer_difference: quality.vtracer_layer_difference(),
        corner_threshold: quality.vtracer_corner_threshold(),
        length_threshold: quality.vtracer_length_threshold(),
        max_iterations: quality.vtracer_max_iterations(),
        splice_threshold: quality.vtracer_splice_threshold(),
        mode: quality.vtracer_path_mode(),
        path_precision: quality.vtracer_path_precision(),
    };

    // Apply anchor_density — controls how many anchor points along curves.
    // Higher density = lower length_threshold = more anchors = smoother curves.
    //   0 = coarse polygons (length_threshold=5.0, iters=2)
    //  50 = balanced        (length_threshold=1.5, iters=14)
    // 100 = maximum density (length_threshold=0.1, iters=25)
    let density = config.anchor_density.clamp(0.0, 100.0);
    let t = density / 100.0; // 0.0 to 1.0
    // Exponential decay: gives good control across the range.
    // At t=0: 5.0, at t=0.5: ~1.1, at t=1.0: 0.1
    vconfig.length_threshold = 0.1 + 4.9 * (1.0 - t).powi(2);
    vconfig.max_iterations = (2.0 + 23.0 * t).round() as usize;

    if config.color_count > 0 {
        vconfig.color_precision = match config.color_count {
            1..=4 => 4,
            5..=8 => 5,
            9..=16 => 6,
            17..=64 => 7,
            _ => 8,
        };
    }

    tracing::debug!(
        "VTracer config: precision={}, speckle={}, layer_diff={}, corner={}, length={:.1}, iters={}, mode={:?}",
        vconfig.color_precision, vconfig.filter_speckle, vconfig.layer_difference,
        vconfig.corner_threshold, vconfig.length_threshold, vconfig.max_iterations, vconfig.mode,
    );

    vconfig
}

/// Extract the fill color from the first `<path` element in SVG text.
fn extract_first_path_fill(svg: &str) -> Option<String> {
    let path_pos = svg.find("<path")?;
    let after_path = &svg[path_pos..];
    let fill_offset = after_path.find("fill=\"")? + 6;
    let end_offset = after_path[fill_offset..].find('"')? + fill_offset;
    Some(after_path[fill_offset..end_offset].to_string())
}

/// Post-process: insert a background `<rect>` as the first child of `<svg>`.
/// Uses the first path's fill color. Fills octagonal corner gaps from
/// vtracer's path simplification on rectangular background regions.
fn postprocess_svg(svg: &str, width: usize, height: usize) -> String {
    let Some(bg_color) = extract_first_path_fill(svg) else {
        return svg.to_string();
    };

    // Find the closing ">" of the <svg ...> opening tag
    let Some(svg_tag_start) = svg.find("<svg") else {
        return svg.to_string();
    };
    let Some(close_offset) = svg[svg_tag_start..].find('>') else {
        return svg.to_string();
    };
    let insert_pos = svg_tag_start + close_offset + 1;

    let rect = format!(
        "\n<rect width=\"{}\" height=\"{}\" fill=\"{}\"/>",
        width, height, bg_color
    );

    let mut result = String::with_capacity(svg.len() + rect.len());
    result.push_str(&svg[..insert_pos]);
    result.push_str(&rect);
    result.push_str(&svg[insert_pos..]);
    result
}

/// Vectorize using the vtracer engine.
pub fn vectorize_with_vtracer(image: &DynamicImage, config: &VectorizeConfig) -> Result<String> {
    // Run our preprocessing for alpha/transparency handling
    let prepared = crate::preprocess::prepare(image, config);

    let width = prepared.width as usize;
    let height = prepared.height as usize;

    if width == 0 || height == 0 {
        return Err(VectorizeError::EmptyImage);
    }

    // Ensure transparent pixels have alpha=0 for vtracer's keying
    let mut rgba = prepared.image;
    if let Some(ref mask) = prepared.opaque_mask {
        for (i, pixel) in rgba.pixels_mut().enumerate() {
            if !mask[i] {
                pixel[0] = 0;
                pixel[1] = 0;
                pixel[2] = 0;
                pixel[3] = 0;
            }
        }
        tracing::debug!(
            "Preprocessed: {} transparent pixels for vtracer keying",
            mask.iter().filter(|&&m| !m).count()
        );
    }

    let color_image = visioncortex::ColorImage {
        pixels: rgba.as_raw().to_vec(),
        width,
        height,
    };

    let vtracer_config = build_vtracer_config(&config.quality, config);
    tracing::info!("Quality: {}", config.quality);

    let svg = vtracer::convert(color_image, vtracer_config)
        .map_err(VectorizeError::TracingFailed)?;

    // Post-process: add background rect to fill octagonal corner gaps
    let mut svg_string = postprocess_svg(&svg.to_string(), width, height);

    // Adaptive curve refinement: smooth curved sections while keeping corners crisp.
    // Applied when curve_smoothness is in the "crisp" range (< 30) — polygon mode
    // or near-polygon output benefits most from selective bezier fitting.
    if let Some(refine_opts) = config.quality.vtracer_refine_options() {
        let t0 = std::time::Instant::now();
        svg_string = crate::refine::refine_svg(&svg_string, &refine_opts);
        tracing::info!("Adaptive curve refinement: {:?}", t0.elapsed());
    }

    Ok(svg_string)
}
