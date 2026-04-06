//! Hybrid backend — vtracer for color clustering + kurbo for curve re-fitting.
//!
//! This engine leverages vtracer's excellent color segmentation and path tracing,
//! then re-fits each path through kurbo's `simplify_bezpath` for cleaner curves.
//! Optionally detects geometric primitives (circles, rectangles) via `crate::shapes`.

use std::sync::atomic::Ordering;
use kurbo::simplify::{simplify_bezpath, SimplifyOptions};
use crate::par::iter_prelude::*;

use crate::{Color, ProgressState, VectorizeConfig, segment};

/// A parsed SVG path element with its fill color, `d` attribute, and transform.
#[derive(Debug, Clone)]
struct ParsedPath {
    d: String,
    fill: Color,
    /// Translation offset from `transform="translate(x,y)"`.
    translate: (f64, f64),
}

/// Result of re-fitting: either a verbatim SVG string or a (BezPath, Color) pair.
#[derive(Debug, Clone)]
enum RefittedElement {
    Verbatim(String),
    Path(kurbo::BezPath, Color),
}

/// An element from the vtracer SVG output — either a rect or a path.
#[derive(Debug, Clone)]
enum SvgElement {
    /// A `<rect .../>` element, kept verbatim.
    Rect(String),
    /// A `<path d="..." fill="..."/>` element, parsed for re-fitting.
    Path(ParsedPath),
}

/// Parse a hex color string `#RRGGBB` into a `Color`.
fn parse_hex_color(s: &str) -> Option<Color> {
    let s = s.strip_prefix('#')?;
    if s.len() != 6 {
        return None;
    }
    let r = u8::from_str_radix(&s[0..2], 16).ok()?;
    let g = u8::from_str_radix(&s[2..4], 16).ok()?;
    let b = u8::from_str_radix(&s[4..6], 16).ok()?;
    Some(Color::rgb(r, g, b))
}

/// Parse an `rgb(r,g,b)` color string into a `Color`.
fn parse_rgb_color(s: &str) -> Option<Color> {
    let s = s.strip_prefix("rgb(")?;
    let s = s.strip_suffix(')')?;
    let parts: Vec<&str> = s.split(',').collect();
    if parts.len() != 3 {
        return None;
    }
    let r: u8 = parts[0].trim().parse().ok()?;
    let g: u8 = parts[1].trim().parse().ok()?;
    let b: u8 = parts[2].trim().parse().ok()?;
    Some(Color::rgb(r, g, b))
}

/// Parse a fill color from either `#RRGGBB` or `rgb(r,g,b)` format.
fn parse_fill_color(s: &str) -> Option<Color> {
    if s.starts_with('#') {
        parse_hex_color(s)
    } else if s.starts_with("rgb(") {
        parse_rgb_color(s)
    } else {
        None
    }
}

/// Extract the value of an attribute from an element string.
/// e.g., `extract_attr(element, "fill")` returns the value between `fill="` and `"`.
fn extract_attr<'a>(element: &'a str, attr: &str) -> Option<&'a str> {
    let needle = format!("{attr}=\"");
    let start = element.find(&needle)? + needle.len();
    let end = start + element[start..].find('"')?;
    Some(&element[start..end])
}

/// Parse the vtracer SVG output into a header (the `<svg ...>` opening tag),
/// a list of elements, and the dimensions.
fn parse_vtracer_svg(svg: &str) -> (String, Vec<SvgElement>) {
    let mut header = String::new();
    let mut elements = Vec::new();

    // Extract everything up to and including the first `>` after `<svg`.
    if let Some(svg_start) = svg.find("<svg") {
        if let Some(close) = svg[svg_start..].find('>') {
            let end = svg_start + close + 1;
            // Include XML declaration and comments before <svg> as well.
            header = svg[..end].to_string();
        }
    }

    // Find all <rect .../> elements.
    let mut search_from = 0;
    while let Some(rect_pos) = svg[search_from..].find("<rect") {
        let abs_pos = search_from + rect_pos;
        // Find the closing /> or >
        if let Some(end_offset) = svg[abs_pos..].find("/>") {
            let full = &svg[abs_pos..abs_pos + end_offset + 2];
            elements.push(SvgElement::Rect(full.to_string()));
            search_from = abs_pos + end_offset + 2;
        } else {
            search_from = abs_pos + 5;
        }
    }

    // Find all <path .../> elements.
    search_from = 0;
    while let Some(path_pos) = svg[search_from..].find("<path") {
        let abs_pos = search_from + path_pos;
        if let Some(end_offset) = svg[abs_pos..].find("/>") {
            let element_str = &svg[abs_pos..abs_pos + end_offset + 2];

            let d = extract_attr(element_str, "d");
            let fill = extract_attr(element_str, "fill");

            let transform = extract_attr(element_str, "transform");
            let translate = parse_translate(transform);

            match (d, fill) {
                (Some(d_val), Some(fill_val)) => {
                    if let Some(color) = parse_fill_color(fill_val) {
                        elements.push(SvgElement::Path(ParsedPath {
                            d: d_val.to_string(),
                            fill: color,
                            translate,
                        }));
                    } else {
                        tracing::warn!(
                            "Hybrid: skipping path with unparseable fill color: {fill_val}"
                        );
                        // Keep the original element as a rect (verbatim passthrough).
                        elements.push(SvgElement::Rect(element_str.to_string()));
                    }
                }
                _ => {
                    tracing::warn!("Hybrid: skipping path missing d or fill attribute");
                    elements.push(SvgElement::Rect(element_str.to_string()));
                }
            }
            search_from = abs_pos + end_offset + 2;
        } else {
            search_from = abs_pos + 5;
        }
    }

    (header, elements)
}

/// Parse a `translate(x,y)` transform into (x, y). Returns (0,0) if missing/invalid.
fn parse_translate(transform: Option<&str>) -> (f64, f64) {
    let Some(s) = transform else {
        return (0.0, 0.0);
    };
    // Match "translate(x,y)" or "translate(x, y)"
    let s = s.trim();
    if let Some(inner) = s.strip_prefix("translate(").and_then(|s| s.strip_suffix(')')) {
        let parts: Vec<&str> = inner.split(',').collect();
        if parts.len() == 2 {
            let x = parts[0].trim().parse::<f64>().unwrap_or(0.0);
            let y = parts[1].trim().parse::<f64>().unwrap_or(0.0);
            return (x, y);
        }
    }
    (0.0, 0.0)
}

/// Apply a translation offset to every point in a BezPath.
fn apply_translate(path: &mut kurbo::BezPath, tx: f64, ty: f64) {
    if tx.abs() < 1e-10 && ty.abs() < 1e-10 {
        return;
    }
    let translate = kurbo::Affine::translate(kurbo::Vec2::new(tx, ty));
    *path = translate * &*path;
}

/// Fix a malformed BezPath: ensure every subpath starts with MoveTo.
/// VTracer can produce paths where ClosePath is followed by LineTo/CurveTo
/// without an intervening MoveTo, which causes kurbo to panic on `.segments()`.
/// We fix this by inserting a MoveTo(last_move) after every ClosePath that's
/// followed by a non-MoveTo element.
fn sanitize_bezpath(path: &mut kurbo::BezPath) {
    let elems = path.elements().to_vec();
    if elems.is_empty() {
        return;
    }

    let mut needs_fix = false;
    for i in 0..elems.len().saturating_sub(1) {
        if matches!(elems[i], kurbo::PathEl::ClosePath)
            && !matches!(elems[i + 1], kurbo::PathEl::MoveTo(_))
        {
            needs_fix = true;
            break;
        }
    }
    if !needs_fix {
        return;
    }

    let mut fixed = kurbo::BezPath::new();
    let mut last_move = kurbo::Point::ZERO;
    for el in &elems {
        match el {
            kurbo::PathEl::MoveTo(p) => {
                last_move = *p;
                fixed.push(*el);
            }
            kurbo::PathEl::ClosePath => {
                fixed.push(*el);
                // Peek: if next element isn't MoveTo, insert one.
                // We'll handle this by always inserting MoveTo after Close,
                // then dedup consecutive MoveTos below.
                fixed.push(kurbo::PathEl::MoveTo(last_move));
            }
            _ => {
                fixed.push(*el);
            }
        }
    }

    // Remove trailing MoveTo if path ends with Close + MoveTo.
    let fixed_elems = fixed.elements();
    if fixed_elems.len() >= 2
        && matches!(fixed_elems[fixed_elems.len() - 1], kurbo::PathEl::MoveTo(_))
        && matches!(fixed_elems[fixed_elems.len() - 2], kurbo::PathEl::ClosePath)
    {
        let mut trimmed = kurbo::BezPath::new();
        for el in &fixed_elems[..fixed_elems.len() - 1] {
            trimmed.push(*el);
        }
        *path = trimmed;
    } else {
        *path = fixed;
    }

    // Remove duplicate consecutive MoveTos (keep last one).
    let elems2 = path.elements().to_vec();
    let mut deduped = kurbo::BezPath::new();
    for i in 0..elems2.len() {
        if matches!(elems2[i], kurbo::PathEl::MoveTo(_))
            && i + 1 < elems2.len()
            && matches!(elems2[i + 1], kurbo::PathEl::MoveTo(_))
        {
            continue; // skip this MoveTo, keep the next one
        }
        deduped.push(elems2[i]);
    }
    *path = deduped;
}

/// Apply Chaikin's corner-cutting subdivision to smooth a BezPath.
/// Each iteration replaces sharp corners with smoother curves.
/// `iterations`: 0 = no smoothing, 1 = mild, 2 = moderate, 3+ = very smooth.
fn chaikin_smooth(path: &kurbo::BezPath, iterations: u32) -> kurbo::BezPath {
    use kurbo::{PathEl, Point};

    if iterations == 0 || path.elements().len() < 3 {
        return path.clone();
    }

    // Extract subpaths, smooth each independently.
    let mut subpaths: Vec<(Vec<Point>, bool)> = Vec::new(); // (points, is_closed)
    let mut current_points: Vec<Point> = Vec::new();
    let mut subpath_start = Point::ZERO;

    for el in path.elements() {
        match *el {
            PathEl::MoveTo(p) => {
                if current_points.len() >= 2 {
                    // Treat as closed — prevents stray lines from unclosed subpaths
                    subpaths.push((current_points, true));
                }
                current_points = vec![p];
                subpath_start = p;
            }
            PathEl::LineTo(p) => current_points.push(p),
            PathEl::QuadTo(_, p2) => current_points.push(p2),
            PathEl::CurveTo(_, _, p3) => current_points.push(p3),
            PathEl::ClosePath => {
                if current_points.len() >= 2 {
                    subpaths.push((current_points, true));
                }
                current_points = Vec::new();
            }
        }
    }
    if current_points.len() >= 2 {
        // Final subpath — always close it
        subpaths.push((current_points, true));
    }

    // Apply Chaikin to each subpath.
    let mut result = kurbo::BezPath::new();
    for (mut points, closed) in subpaths {
        for _ in 0..iterations {
            points = chaikin_step(&points, closed);
        }
        if points.is_empty() {
            continue;
        }
        result.move_to(points[0]);
        for p in &points[1..] {
            result.line_to(*p);
        }
        if closed {
            result.close_path();
        }
    }

    // Re-simplify the smoothed polyline back into bezier curves.
    if result.elements().len() > 2 {
        let opts = SimplifyOptions::default().angle_thresh(0.01);
        simplify_bezpath(result.elements().iter().copied(), 0.5, &opts)
    } else {
        result
    }
}

/// One iteration of selective Chaikin's corner-cutting.
/// Only smooths corners where the turning angle is SMALL (pixel stairs).
/// Preserves corners where the angle is large (actual shape features).
/// `max_smooth_angle_deg`: only smooth corners below this angle.
fn chaikin_step(points: &[kurbo::Point], closed: bool) -> Vec<kurbo::Point> {
    use kurbo::Point;

    if points.len() < 3 {
        return points.to_vec();
    }

    let n = points.len();
    let mut smoothed = Vec::with_capacity(n * 2);
    let max_smooth_angle = 30.0_f64; // Only smooth turns < 30°

    let range = if closed { n } else { n - 1 };

    if !closed {
        smoothed.push(points[0]);
    }

    for i in 0..range {
        let p0 = points[i];
        let p1 = points[(i + 1) % n];

        // Check turning angle at p1 (the "corner" between this segment and the next).
        let should_smooth = if i + 1 < range || closed {
            let p2 = points[(i + 2) % n];
            let v1x = p1.x - p0.x;
            let v1y = p1.y - p0.y;
            let v2x = p2.x - p1.x;
            let v2y = p2.y - p1.y;
            let len1 = (v1x * v1x + v1y * v1y).sqrt();
            let len2 = (v2x * v2x + v2y * v2y).sqrt();
            if len1 > 1e-9 && len2 > 1e-9 {
                let cos_a = (v1x * v2x + v1y * v2y) / (len1 * len2);
                let angle = cos_a.clamp(-1.0, 1.0).acos().to_degrees();
                angle < max_smooth_angle // Only smooth small turns (pixel stairs)
            } else {
                false
            }
        } else {
            false
        };

        if should_smooth {
            // Chaikin: replace segment endpoints with 25%/75% points
            let q = Point::new(0.75 * p0.x + 0.25 * p1.x, 0.75 * p0.y + 0.25 * p1.y);
            let r = Point::new(0.25 * p0.x + 0.75 * p1.x, 0.25 * p0.y + 0.75 * p1.y);
            smoothed.push(q);
            smoothed.push(r);
        } else {
            // Keep original points — this is a real corner
            smoothed.push(p0);
            smoothed.push(p1);
        }
    }

    if !closed {
        smoothed.push(points[n - 1]);
    }

    // Deduplicate consecutive near-identical points
    let mut deduped: Vec<Point> = Vec::with_capacity(smoothed.len());
    for p in &smoothed {
        if let Some(last) = deduped.last() {
            let dx = p.x - last.x;
            let dy = p.y - last.y;
            if dx * dx + dy * dy < 0.01 {
                continue;
            }
        }
        deduped.push(*p);
    }

    deduped
}

/// Maximum elements per chunk for simplify_bezpath.
/// Large paths are split into chunks this size to avoid kurbo's O(n^3) hang.
const MAX_SIMPLIFY_CHUNK: usize = 60;

/// Re-fit a single path using kurbo's simplify, returning the simplified path.
/// Returns `None` if parsing fails.
fn refit_path(d: &str, tolerance: f64, angle_thresh: f64, smooth_iters: u32, translate: (f64, f64)) -> Option<kurbo::BezPath> {
    let mut bez = kurbo::BezPath::from_svg(d).ok()?;
    if bez.elements().is_empty() {
        return Some(bez);
    }
    // Apply the translate transform so coordinates are in absolute space
    apply_translate(&mut bez, translate.0, translate.1);

    // Fix malformed paths from vtracer (ClosePath not followed by MoveTo).
    sanitize_bezpath(&mut bez);

    let elem_count = bez.elements().len();

    // Scale tolerance based on element count:
    // Small paths (< 50 elements) are fine features — tighter fit.
    // Large paths (> 200 elements) are broad regions — more smoothing OK.
    let tolerance = if elem_count < 50 {
        tolerance * 0.5
    } else if elem_count > 200 {
        tolerance * 1.5
    } else {
        tolerance
    };

    // Apply Chaikin smoothing first if requested (pixel staircase removal)
    if smooth_iters > 0 {
        bez = chaikin_smooth(&bez, smooth_iters);
    }

    // Now apply kurbo simplify_bezpath for proper bezier fitting.
    // Split large paths into chunks to avoid kurbo's O(n^3) runtime.
    let simplified = simplify_path_safe(&bez, tolerance, angle_thresh);

    let mut result = simplified;
    ensure_all_subpaths_closed(&mut result);
    Some(result)
}

/// Safely simplify a BezPath by splitting into sub-paths and chunking large ones.
/// kurbo's simplify_bezpath can hang on paths with >60 elements due to O(n^3)
/// behavior. This function splits at natural sub-path boundaries first, then
/// chunks any sub-path that exceeds MAX_SIMPLIFY_CHUNK.
fn simplify_path_safe(path: &kurbo::BezPath, tolerance: f64, angle_thresh: f64) -> kurbo::BezPath {
    let opts = SimplifyOptions::default().angle_thresh(angle_thresh);

    // Split into natural sub-paths (at MoveTo boundaries)
    let subpaths = split_into_subpaths(path);
    let mut result = kurbo::BezPath::new();

    for subpath in &subpaths {
        let elem_count = subpath.elements().len();

        if elem_count <= MAX_SIMPLIFY_CHUNK {
            // Small enough — simplify directly (with panic guard)
            let simplified = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                simplify_bezpath(subpath.elements().iter().copied(), tolerance, &opts)
            }));
            match simplified {
                Ok(s) => {
                    for el in s.elements() {
                        result.push(*el);
                    }
                }
                Err(_) => {
                    // Simplify panicked — use original sub-path
                    for el in subpath.elements() {
                        result.push(*el);
                    }
                }
            }
        } else {
            // Too large — flatten to polyline, chunk, simplify each chunk, rejoin
            let elements = subpath.elements().to_vec();
            let mut chunk_start = 0;

            while chunk_start < elements.len() {
                let chunk_end = (chunk_start + MAX_SIMPLIFY_CHUNK).min(elements.len());

                // Build chunk BezPath
                let mut chunk = kurbo::BezPath::new();
                for el in &elements[chunk_start..chunk_end] {
                    chunk.push(*el);
                }

                // Ensure chunk starts with MoveTo
                if !chunk.elements().is_empty() {
                    if !matches!(chunk.elements()[0], kurbo::PathEl::MoveTo(_)) {
                        // Prepend a MoveTo from the previous element's endpoint
                        let start_pt = if chunk_start > 0 {
                            endpoint_of(&elements[chunk_start - 1])
                        } else {
                            kurbo::Point::ZERO
                        };
                        let mut fixed = kurbo::BezPath::new();
                        fixed.move_to(start_pt);
                        for el in chunk.elements() {
                            fixed.push(*el);
                        }
                        chunk = fixed;
                    }
                }

                let simplified = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    simplify_bezpath(chunk.elements().iter().copied(), tolerance, &opts)
                }));

                match simplified {
                    Ok(s) => {
                        // Skip the MoveTo of subsequent chunks to avoid gaps
                        let skip = if chunk_start > 0 { 1 } else { 0 };
                        for el in s.elements().iter().skip(skip) {
                            result.push(*el);
                        }
                    }
                    Err(_) => {
                        let skip = if chunk_start > 0 { 1 } else { 0 };
                        for el in chunk.elements().iter().skip(skip) {
                            result.push(*el);
                        }
                    }
                }

                chunk_start = chunk_end;
            }
        }
    }

    result
}

/// Split a BezPath into individual sub-paths (one per MoveTo).
fn split_into_subpaths(path: &kurbo::BezPath) -> Vec<kurbo::BezPath> {
    let mut subpaths = Vec::new();
    let mut current = kurbo::BezPath::new();

    for el in path.elements() {
        if matches!(el, kurbo::PathEl::MoveTo(_)) && !current.elements().is_empty() {
            subpaths.push(current);
            current = kurbo::BezPath::new();
        }
        current.push(*el);
    }

    if !current.elements().is_empty() {
        subpaths.push(current);
    }

    subpaths
}

/// Get the endpoint of a path element.
fn endpoint_of(el: &kurbo::PathEl) -> kurbo::Point {
    match *el {
        kurbo::PathEl::MoveTo(p) | kurbo::PathEl::LineTo(p) => p,
        kurbo::PathEl::QuadTo(_, p) => p,
        kurbo::PathEl::CurveTo(_, _, p) => p,
        kurbo::PathEl::ClosePath => kurbo::Point::ZERO,
    }
}

/// Ensure every subpath in a BezPath ends with ClosePath.
/// Prevents stray diagonal lines in the rendered SVG.
fn ensure_all_subpaths_closed(path: &mut kurbo::BezPath) {
    let elements = path.elements().to_vec();
    let mut fixed = kurbo::BezPath::new();
    let mut has_content = false;

    for (i, el) in elements.iter().enumerate() {
        match *el {
            kurbo::PathEl::MoveTo(p) => {
                // Close previous subpath if it wasn't closed
                if has_content {
                    if i > 0 && !matches!(elements[i - 1], kurbo::PathEl::ClosePath) {
                        fixed.close_path();
                    }
                }
                fixed.move_to(p);
                has_content = false;
            }
            kurbo::PathEl::LineTo(p) => { fixed.line_to(p); has_content = true; }
            kurbo::PathEl::QuadTo(c1, p) => { fixed.quad_to(c1, p); has_content = true; }
            kurbo::PathEl::CurveTo(c1, c2, p) => { fixed.curve_to(c1, c2, p); has_content = true; }
            kurbo::PathEl::ClosePath => { fixed.close_path(); has_content = false; }
        }
    }

    // Close final subpath
    if has_content {
        if !matches!(elements.last(), Some(kurbo::PathEl::ClosePath)) {
            fixed.close_path();
        }
    }

    *path = fixed;
}

/// Vectorize using the hybrid engine: vtracer clustering + kurbo curve re-fitting.
///
/// 1. Run vtracer to get an initial SVG with color-clustered paths.
/// 2. Parse each `<path>` element and re-fit through kurbo's simplifier.
/// 3. Optionally detect geometric primitives (circles, rectangles).
/// 4. Re-serialize to clean SVG output.
pub fn vectorize_hybrid(
    image: &image::DynamicImage,
    config: &VectorizeConfig,
) -> crate::Result<String> {
    // No progress tracking — use a dummy state.
    let state = ProgressState::new();
    vectorize_hybrid_with_progress(image, config, &state)
}

/// Hybrid engine with shared progress state (read by UI independently).
pub fn vectorize_hybrid_with_progress(
    image: &image::DynamicImage,
    config: &VectorizeConfig,
    state: &ProgressState,
) -> crate::Result<String> {
    // Step 0: Pre-quantize using Oklab color science.
    // Color Detail slider directly controls the palette size (4→256 colors).
    let vtracer_image = {
        state.stage.store(0, Ordering::Relaxed);
        let rgba = image.to_rgba8();
        let mut prequant_config = config.clone();
        if prequant_config.color_count == 0 {
            prequant_config.color_count = config.quality.auto_color_count_hint();
        }
        match segment::quantize_rgba_image(&rgba, &prequant_config) {
            Ok(segmented) => {
                tracing::info!(
                    "Hybrid: pre-quantized to {} Oklab colors (color_detail={:.0})",
                    segmented.palette.len(),
                    config.quality.color_detail,
                );
                image::DynamicImage::ImageRgba8(segment::render_quantized_image(&segmented))
            }
            Err(e) => {
                tracing::warn!("Hybrid: pre-quantization failed ({e}), using original");
                image.clone()
            }
        }
    };

    // Step 1: Get vtracer's SVG output.
    // Wrap in catch_unwind — visioncortex can panic on integer overflow
    // with certain image/settings combinations (e.g., color.rs:273 add overflow).
    state.stage.store(0, Ordering::Relaxed); // "Color clustering..."
    let vtracer_result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        super::vtracer_backend::vectorize_with_vtracer(&vtracer_image, config)
    }));
    let vtracer_svg = match vtracer_result {
        Ok(Ok(svg)) => svg,
        Ok(Err(e)) => return Err(e),
        Err(_) => {
            return Err(crate::VectorizeError::TracingFailed(
                "vtracer panicked (likely integer overflow — try reducing color detail or image size)".into(),
            ));
        }
    };

    if state.cancelled.load(Ordering::Relaxed) {
        return Err(crate::VectorizeError::TracingFailed("Cancelled".into()));
    }

    // Step 2: Parse the SVG.
    let (header, elements) = parse_vtracer_svg(&vtracer_svg);
    let total = elements.len();

    let tolerance = config.quality.hybrid_refit_tolerance();
    let angle_thresh = config.quality.hybrid_angle_thresh();
    let smooth_iters = config.quality.hybrid_smooth_iterations();
    let shape_tolerance = config.quality.shape_detection_tolerance();
    let detect_shapes = config.detect_shapes;

    tracing::debug!(
        "Hybrid: {} elements parsed, refit_tolerance={:.2}, angle_thresh={:.4}, detect_shapes={}",
        total,
        tolerance,
        angle_thresh,
        detect_shapes,
    );

    // Step 3: Re-fit paths in parallel with a global time budget.
    // If the refit phase exceeds 10 seconds, remaining paths are kept as-is.
    state.stage.store(1, Ordering::Relaxed); // "Refitting paths"
    state.total.store(total, Ordering::Relaxed);
    state.current.store(0, Ordering::Relaxed);

    let refit_deadline = crate::par::instant_now() + std::time::Duration::from_secs(10);
    let deadline_exceeded = std::sync::atomic::AtomicBool::new(false);

    let refitted: Vec<RefittedElement> = crate::par::maybe_par_iter!(elements)
        .map(|elem| {
            // Check global deadline + cancellation
            if deadline_exceeded.load(Ordering::Relaxed)
                || state.cancelled.load(Ordering::Relaxed)
            {
                // Time's up — keep path as-is without refitting
                return match elem {
                    SvgElement::Rect(raw) => RefittedElement::Verbatim(raw.clone()),
                    SvgElement::Path(parsed) => {
                        if let Ok(mut bez) = kurbo::BezPath::from_svg(&parsed.d) {
                            apply_translate(&mut bez, parsed.translate.0, parsed.translate.1);
                            sanitize_bezpath(&mut bez);
                            RefittedElement::Path(bez, parsed.fill)
                        } else {
                            let fill_str = parsed.fill.to_svg_color();
                            RefittedElement::Verbatim(format!(
                                "<path d=\"{}\" fill=\"{fill_str}\"/>",
                                parsed.d
                            ))
                        }
                    }
                };
            }

            if crate::par::instant_now() >= refit_deadline {
                deadline_exceeded.store(true, Ordering::Relaxed);
                tracing::warn!("Hybrid: refit phase exceeded 10s budget, skipping remaining paths");
            }

            // Catch any panics from kurbo (malformed paths) or shape detection.
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                match elem {
                    SvgElement::Rect(raw) => RefittedElement::Verbatim(raw.clone()),
                    SvgElement::Path(parsed) => {
                        match refit_path(&parsed.d, tolerance, angle_thresh, smooth_iters, parsed.translate) {
                            Some(refitted) => {
                                if detect_shapes {
                                    if let Some(shape_svg) =
                                        try_detect_shape(&refitted, &parsed.fill, shape_tolerance)
                                    {
                                        return RefittedElement::Verbatim(shape_svg);
                                    }
                                }
                                RefittedElement::Path(refitted, parsed.fill)
                            }
                            None => {
                                let fill_str = parsed.fill.to_svg_color();
                                RefittedElement::Verbatim(format!(
                                    "<path d=\"{}\" fill=\"{fill_str}\"/>",
                                    parsed.d
                                ))
                            }
                        }
                    }
                }
            }));
            state.current.fetch_add(1, Ordering::Relaxed);
            match result {
                Ok(elem) => elem,
                Err(_) => {
                    // Path caused a panic — parse, sanitize, and keep as a Path
                    // so it still goes through merge (with translate applied).
                    match elem {
                        SvgElement::Rect(raw) => RefittedElement::Verbatim(raw.clone()),
                        SvgElement::Path(parsed) => {
                            if let Ok(mut bez) = kurbo::BezPath::from_svg(&parsed.d) {
                                apply_translate(&mut bez, parsed.translate.0, parsed.translate.1);
                                sanitize_bezpath(&mut bez);
                                RefittedElement::Path(bez, parsed.fill)
                            } else {
                                let fill_str = parsed.fill.to_svg_color();
                                RefittedElement::Verbatim(format!(
                                    "<path d=\"{}\" fill=\"{fill_str}\"/>",
                                    parsed.d
                                ))
                            }
                        }
                    }
                }
            }
        })
        .collect();

    if state.cancelled.load(Ordering::Relaxed) {
        return Err(crate::VectorizeError::TracingFailed("Cancelled".into()));
    }

    // Separate verbatim elements and refitted paths.
    let mut verbatim_elements: Vec<String> = Vec::new();
    let mut path_pairs: Vec<(kurbo::BezPath, Color)> = Vec::new();

    for elem in refitted {
        match elem {
            RefittedElement::Verbatim(s) => verbatim_elements.push(s),
            RefittedElement::Path(mut bez, color) => {
                if !bez.elements().is_empty() {
                    // Ensure the path ends with ClosePath to prevent
                    // stray diagonal lines when paths are merged.
                    if let Some(last) = bez.elements().last() {
                        if !matches!(last, kurbo::PathEl::ClosePath) {
                            bez.close_path();
                        }
                    }
                    path_pairs.push((bez, color));
                }
            }
        }
    }

    // Step 4b: Palette reduction — limit tones per hue group.
    if config.tones_per_hue > 0 {
        let colors: Vec<Color> = path_pairs.iter().map(|(_, c)| *c).collect();
        let remap = crate::palette::reduce_palette(&colors, config.tones_per_hue as usize, 12);
        if !remap.is_empty() {
            crate::palette::apply_palette_reduction(&mut path_pairs, &remap);
            let unique_before = colors.iter().collect::<std::collections::HashSet<_>>().len();
            let unique_after = path_pairs.iter().map(|(_, c)| c).collect::<std::collections::HashSet<_>>().len();
            tracing::info!("Hybrid: palette reduced {} → {} unique colors (tones_per_hue={})",
                unique_before, unique_after, config.tones_per_hue);
        }
    }

    // Step 5: Post-process — merge same-color paths and detect strokes.
    state.stage.store(2, Ordering::Relaxed); // "Merging paths..."
    let merge_paths = config.merge_paths;
    tracing::debug!("Hybrid: merge_paths={}, {} path pairs to process", merge_paths, path_pairs.len());
    let output_path_elements: Vec<String> = if merge_paths {
        let color_tol = config.quality.merge_color_tolerance();
        // Wrap merge in catch_unwind — classify_path calls .segments()/.area() which
        // can still panic on edge-case paths despite sanitization.
        let merge_result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            crate::merge::post_process(&path_pairs, color_tol)
        }));
        let merged = match merge_result {
            Ok(m) => m,
            Err(_) => {
                tracing::warn!("Hybrid: merge panicked, skipping merge step");
                // Fall back: each path becomes a filled element, no merging.
                path_pairs.iter().map(|(path, color)| {
                    crate::merge::MergedPath::Filled { path: path.clone(), color: *color }
                }).collect()
            }
        };
        tracing::info!("Hybrid: merged {} paths into {} elements ({} strokes)",
            path_pairs.len(), merged.len(),
            merged.iter().filter(|m| matches!(m, crate::merge::MergedPath::Stroked { .. })).count()
        );
        merged
            .iter()
            .map(|mp| match mp {
                crate::merge::MergedPath::Filled { path, color } => {
                    let fill_str = color.to_svg_color();
                    let d_str = crate::output::bezpath_to_svg_d(path);
                    format!("<path d=\"{d_str}\" fill=\"{fill_str}\"/>")
                }
                crate::merge::MergedPath::Stroked {
                    path,
                    color,
                    width,
                } => {
                    let color_str = color.to_svg_color();
                    let d_str = crate::output::bezpath_to_svg_d(path);
                    format!(
                        "<path d=\"{d_str}\" stroke=\"{color_str}\" stroke-width=\"{width:.2}\" fill=\"none\" stroke-linecap=\"round\" stroke-linejoin=\"round\"/>"
                    )
                }
            })
            .collect()
    } else {
        path_pairs
            .iter()
            .map(|(path, color)| {
                let fill_str = color.to_svg_color();
                let d_str = crate::output::bezpath_to_svg_d(path);
                format!("<path d=\"{d_str}\" fill=\"{fill_str}\"/>")
            })
            .collect()
    };

    // Filter out empty paths (from panic recovery or empty merges).
    let output_path_elements: Vec<String> = output_path_elements
        .into_iter()
        .filter(|s| !s.contains("d=\"\""))
        .collect();

    // Step 6: Re-serialize to SVG.
    state.stage.store(3, Ordering::Relaxed); // "Building SVG..."
    let total_elements = verbatim_elements.len() + output_path_elements.len();
    let mut svg = String::with_capacity(vtracer_svg.len());
    svg.push_str(&header);
    svg.push('\n');
    for elem in &verbatim_elements {
        svg.push_str(elem);
        svg.push('\n');
    }
    for elem in &output_path_elements {
        svg.push_str(elem);
        svg.push('\n');
    }
    svg.push_str("</svg>");

    tracing::info!(
        "Hybrid: output {} elements ({} merged paths), {} bytes",
        total_elements,
        output_path_elements.len(),
        svg.len(),
    );

    Ok(svg)
}

/// Try to detect a geometric primitive from the refitted path.
/// Returns an SVG element string if a primitive was detected, `None` if the path
/// is not a recognized shape (i.e., `DetectedShape::Path`).
fn try_detect_shape(path: &kurbo::BezPath, color: &Color, tolerance: f64) -> Option<String> {
    let shape = crate::shapes::detect_shape(path, tolerance);
    let fill_str = color.to_svg_color();
    match shape {
        crate::shapes::DetectedShape::Path(_) => None,
        _ => Some(crate::shapes::shape_to_svg(&shape, &fill_str, "")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_hex_color() {
        let c = parse_hex_color("#6B7178").unwrap();
        assert_eq!(c.r, 0x6B);
        assert_eq!(c.g, 0x71);
        assert_eq!(c.b, 0x78);
        assert_eq!(c.a, 255);
    }

    #[test]
    fn test_parse_hex_color_lowercase() {
        let c = parse_hex_color("#ff00ab").unwrap();
        assert_eq!(c.r, 0xFF);
        assert_eq!(c.g, 0x00);
        assert_eq!(c.b, 0xAB);
    }

    #[test]
    fn test_parse_hex_color_invalid() {
        assert!(parse_hex_color("#GG0000").is_none());
        assert!(parse_hex_color("#12345").is_none());
        assert!(parse_hex_color("6B7178").is_none());
        assert!(parse_hex_color("").is_none());
    }

    #[test]
    fn test_parse_rgb_color() {
        let c = parse_rgb_color("rgb(107,113,120)").unwrap();
        assert_eq!(c.r, 107);
        assert_eq!(c.g, 113);
        assert_eq!(c.b, 120);
    }

    #[test]
    fn test_parse_rgb_color_with_spaces() {
        let c = parse_rgb_color("rgb( 10 , 20 , 30 )").unwrap();
        assert_eq!(c.r, 10);
        assert_eq!(c.g, 20);
        assert_eq!(c.b, 30);
    }

    #[test]
    fn test_parse_rgb_color_invalid() {
        assert!(parse_rgb_color("rgb(256,0,0)").is_none());
        assert!(parse_rgb_color("rgb(1,2)").is_none());
        assert!(parse_rgb_color("hsl(0,0,0)").is_none());
    }

    #[test]
    fn test_parse_fill_color_dispatch() {
        assert!(parse_fill_color("#FF0000").is_some());
        assert!(parse_fill_color("rgb(255,0,0)").is_some());
        assert!(parse_fill_color("blue").is_none());
    }

    #[test]
    fn test_extract_attr() {
        let elem = r##"<path d="M0 0 L10 10" fill="#FF0000" transform="translate(0,0)"/>"##;
        assert_eq!(extract_attr(elem, "d"), Some("M0 0 L10 10"));
        assert_eq!(extract_attr(elem, "fill"), Some("#FF0000"));
        assert_eq!(
            extract_attr(elem, "transform"),
            Some("translate(0,0)")
        );
        assert_eq!(extract_attr(elem, "stroke"), None);
    }

    #[test]
    fn test_parse_vtracer_svg_elements() {
        let svg = r##"<?xml version="1.0" encoding="UTF-8"?>
<!-- Generator: visioncortex VTracer 0.6.5 -->
<svg version="1.1" xmlns="http://www.w3.org/2000/svg" width="100" height="100">
<rect width="100" height="100" fill="#6B7178"/>
<path d="M0 0 L10 10 L10 0 Z" fill="#6B7178" transform="translate(0,0)"/>
<path d="M5 5 L15 15 L15 5 Z" fill="#8A9199" transform="translate(0,0)"/>
</svg>"##;

        let (header, elements) = parse_vtracer_svg(svg);
        assert!(header.contains("<svg"));
        assert!(header.contains("width=\"100\""));

        // Should have 1 rect + 2 paths = 3 elements
        assert_eq!(elements.len(), 3);

        match &elements[0] {
            SvgElement::Rect(r) => assert!(r.contains("rect")),
            _ => panic!("expected rect"),
        }
        match &elements[1] {
            SvgElement::Path(p) => {
                assert_eq!(p.d, "M0 0 L10 10 L10 0 Z");
                assert_eq!(p.fill.r, 0x6B);
            }
            _ => panic!("expected path"),
        }
        match &elements[2] {
            SvgElement::Path(p) => {
                assert_eq!(p.d, "M5 5 L15 15 L15 5 Z");
                assert_eq!(p.fill.r, 0x8A);
            }
            _ => panic!("expected path"),
        }
    }

    #[test]
    fn test_parse_vtracer_svg_with_rgb_fill() {
        let svg = r##"<svg version="1.1" xmlns="http://www.w3.org/2000/svg" width="50" height="50">
<path d="M0 0 L5 5 Z" fill="rgb(255,128,0)" transform="translate(0,0)"/>
</svg>"##;

        let (_header, elements) = parse_vtracer_svg(svg);
        assert_eq!(elements.len(), 1);
        match &elements[0] {
            SvgElement::Path(p) => {
                assert_eq!(p.fill.r, 255);
                assert_eq!(p.fill.g, 128);
                assert_eq!(p.fill.b, 0);
            }
            _ => panic!("expected path"),
        }
    }

    #[test]
    fn test_refit_path_simple() {
        let d = "M0 0 L10 0 L10 10 L0 10 Z";
        let result = refit_path(d, 1.0, 0.1, 0, (0.0, 0.0));
        assert!(result.is_some());
        let path = result.unwrap();
        assert!(!path.elements().is_empty());
    }

    #[test]
    fn test_refit_path_invalid() {
        let result = refit_path("not a valid path", 1.0, 0.1, 0, (0.0, 0.0));
        assert!(result.is_none());
    }

    #[test]
    fn test_refit_path_empty() {
        let result = refit_path("", 1.0, 0.1, 0, (0.0, 0.0));
        // Empty string may or may not parse; either way should not panic.
        // kurbo::BezPath::from_svg("") returns Ok with an empty path.
        if let Some(path) = result {
            assert!(path.elements().is_empty());
        }
    }

    #[test]
    fn test_refit_path_complex_curve() {
        // A cubic bezier path
        let d = "M10 80 C40 10, 65 10, 95 80";
        let result = refit_path(d, 0.5, 0.1, 0, (0.0, 0.0));
        assert!(result.is_some());
        let path = result.unwrap();
        // Should produce a valid path with elements
        assert!(!path.elements().is_empty());
        // The SVG output should be parseable
        let svg_d = path.to_svg();
        assert!(!svg_d.is_empty());
    }

    #[test]
    fn test_parse_vtracer_svg_missing_attrs() {
        // Path without fill should be kept as verbatim fallback
        let svg = r##"<svg version="1.1" xmlns="http://www.w3.org/2000/svg" width="50" height="50">
<path d="M0 0 L5 5 Z"/>
</svg>"##;

        let (_header, elements) = parse_vtracer_svg(svg);
        assert_eq!(elements.len(), 1);
        // Should fall through to Rect (verbatim passthrough) since fill is missing
        match &elements[0] {
            SvgElement::Rect(_) => {} // expected
            _ => panic!("expected verbatim passthrough for path missing fill"),
        }
    }

    #[test]
    fn test_parse_vtracer_svg_bad_fill() {
        let svg = r##"<svg version="1.1" xmlns="http://www.w3.org/2000/svg" width="50" height="50">
<path d="M0 0 L5 5 Z" fill="blue"/>
</svg>"##;

        let (_header, elements) = parse_vtracer_svg(svg);
        assert_eq!(elements.len(), 1);
        match &elements[0] {
            SvgElement::Rect(_) => {} // expected — named colors not supported
            _ => panic!("expected verbatim passthrough for unsupported fill format"),
        }
    }

    #[test]
    fn test_header_includes_xml_declaration() {
        let svg = r##"<?xml version="1.0" encoding="UTF-8"?>
<svg version="1.1" xmlns="http://www.w3.org/2000/svg" width="100" height="100">
<path d="M0 0 L1 1 Z" fill="#000000"/>
</svg>"##;

        let (header, _) = parse_vtracer_svg(svg);
        assert!(header.starts_with("<?xml"));
        assert!(header.ends_with('>'));
    }

    #[test]
    fn test_color_round_trip() {
        let original = "#6b7178";
        let color = parse_hex_color(original).unwrap();
        let svg_str = color.to_svg_color();
        assert_eq!(svg_str, "#6b7178");
    }
}
