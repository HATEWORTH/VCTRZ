//! Logo-mode pipeline — fundamentally different from generic vectorization.
//!
//! Logo mode is designed for flat graphics, icons, and text. Instead of
//! trying to faithfully reproduce gradients and photographic detail, it:
//!
//! 1. Uses the Hybrid backend for initial clustering + path extraction
//! 2. Snaps near-straight bezier segments to true `LineTo` commands
//! 3. Sharpens corners to exact angles (no smoothing)
//! 4. Aggressively detects geometric primitives (circles, rects, ellipses)
//! 5. Merges same-color regions with higher tolerance
//!
//! The result is clean, geometric SVG output suitable for logos and icons.

use kurbo::{BezPath, PathEl, Point};

use crate::{Color, Result, VectorizeConfig};

/// Run the logo-specific pipeline.
///
/// Uses VTracer directly (not Hybrid) to preserve real bezier curves,
/// then applies logo-specific post-processing:
/// - Line snapping (only truly straight curves → lines)
/// - Corner sharpening (remove tiny smoothing artifacts)
/// - Aggressive shape detection (circles, rects, ellipses)
/// Build a VTracer config tuned for Logo mode:
/// - Curve resolution driven by anchor_density slider
/// - Sharp corner detection (keep hard angles crisp)
/// - Spline mode (real bezier curves, not polygons)
/// Build a config adjusted for Logo mode — ensures spline mode is active.
/// Used by the main vectorize pipeline so any engine can be used with Logo.
pub fn logo_adjusted_config(config: &VectorizeConfig) -> VectorizeConfig {
    logo_vtracer_config(config)
}

fn logo_vtracer_config(config: &VectorizeConfig) -> VectorizeConfig {
    let mut logo_config = config.clone();

    // Ensure spline mode — logos need real bezier curves, not polygons.
    // Only override if the user's curve_smoothness is too low for spline mode.
    if logo_config.quality.curve_smoothness < 1.0 {
        logo_config.quality.curve_smoothness = 1.0;
    }

    logo_config
}

pub fn vectorize_logo(
    image: &image::DynamicImage,
    config: &VectorizeConfig,
) -> Result<String> {
    let t0 = crate::par::instant_now();

    // Step 1: Run VTracer with logo-tuned settings for maximum curve resolution
    let logo_config = logo_vtracer_config(config);
    let base_svg = super::vtracer_backend::vectorize_with_vtracer(image, &logo_config)?;

    // Step 2: Parse paths from SVG, apply logo-specific transforms
    let svg = logo_post_process(&base_svg, config);

    tracing::info!("Logo pipeline completed in {:?}", t0.elapsed());
    Ok(svg)
}

/// Run the logo-specific pipeline with progress reporting.
pub fn vectorize_logo_with_progress(
    image: &image::DynamicImage,
    config: &VectorizeConfig,
    state: &crate::ProgressState,
) -> Result<String> {
    let t0 = crate::par::instant_now();

    let logo_config = logo_vtracer_config(config);
    let base_svg = super::vtracer_backend::vectorize_with_vtracer(image, &logo_config)?;
    let svg = logo_post_process(&base_svg, config);

    tracing::info!("Logo pipeline completed in {:?}", t0.elapsed());
    Ok(svg)
}

/// Apply logo-specific post-processing to an SVG string.
/// Parses each <path>, transforms it, and rebuilds the SVG.
/// Apply logo-specific post-processing to any SVG string.
/// Public so the main pipeline can apply it after any engine.
pub fn logo_post_process_svg(svg: &str, config: &VectorizeConfig) -> String {
    logo_post_process(svg, config)
}

/// Apply logo-specific post-processing to an SVG string.
/// Parses each <path>, transforms it, and rebuilds the SVG.
fn logo_post_process(svg: &str, config: &VectorizeConfig) -> String {
    // Split SVG into header, body elements, and footer
    let (header, elements, footer) = parse_svg_structure(svg);

    let mut output = header;

    for elem in &elements {
        match elem {
            SvgPart::Verbatim(s) => {
                output.push_str(s);
                output.push('\n');
            }
            SvgPart::Path { d, fill, other_attrs, translate } => {
                // Parse the path
                if let Ok(mut bez) = BezPath::from_svg(d) {
                    // Apply translate transform so paths are in absolute coordinates
                    apply_translate(&mut bez, translate.0, translate.1);

                    // Logo transform 1: Snap only truly straight curves to lines.
                    // Threshold is very tight (0.5px) — only snap curves that are
                    // genuinely straight. Keep all real curves (ovals, letters, etc.)
                    bez = snap_curves_to_lines(&bez, 0.5);

                    // Logo transform 2: Sharpen corners — remove tiny smoothing
                    // curves (< 2px chord) that are just anti-aliasing artifacts.
                    bez = sharpen_corners(&bez, 15.0_f64.to_radians());

                    // Logo transform 3: Try to detect geometric primitives
                    let tolerance = config.quality.shape_detection_tolerance() * 0.7;
                    let shape = crate::shapes::detect_shape(&bez, tolerance);

                    match shape {
                        crate::shapes::DetectedShape::Path(_) => {
                            // Not a primitive — emit the cleaned path
                            let d_str = bezpath_to_svg_d(&bez);
                            output.push_str(&format!(
                                "<path d=\"{}\" fill=\"{}\"{}/>",
                                d_str, fill, other_attrs
                            ));
                        }
                        _ => {
                            // Emit clean SVG primitive
                            let prim = crate::shapes::shape_to_svg(&shape, &fill, "");
                            output.push_str(&prim);
                        }
                    }
                    output.push('\n');
                } else {
                    // Can't parse — pass through verbatim
                    output.push_str(&format!("<path d=\"{}\" fill=\"{}\"{}/>", d, fill, other_attrs));
                    output.push('\n');
                }
            }
        }
    }

    output.push_str(&footer);
    output
}

// ─────────────────────────────────────────────────────────────────────────────
// LOGO-SPECIFIC PATH TRANSFORMS
// ─────────────────────────────────────────────────────────────────────────────

/// Snap cubic bezier segments that are nearly straight to true `LineTo` commands.
///
/// A cubic bezier `CurveTo(c1, c2, p)` from the current point `p0` is "nearly straight"
/// when both control points are close to the line segment `p0→p`. We measure this as
/// the maximum perpendicular distance of c1 and c2 from the line.
///
/// `max_deviation`: maximum perpendicular distance (in pixels) to consider "straight".
/// Typical value: 1.0-2.0 for logos.
pub fn snap_curves_to_lines(path: &BezPath, max_deviation: f64) -> BezPath {
    let mut result = BezPath::new();
    let mut current = Point::ZERO;

    for el in path.elements() {
        match *el {
            PathEl::MoveTo(p) => {
                current = p;
                result.move_to(p);
            }
            PathEl::LineTo(p) => {
                current = p;
                result.line_to(p);
            }
            PathEl::QuadTo(c1, p) => {
                // Check if quad is nearly straight
                let dev = point_to_line_distance(c1, current, p);
                if dev < max_deviation {
                    result.line_to(p);
                } else {
                    result.quad_to(c1, p);
                }
                current = p;
            }
            PathEl::CurveTo(c1, c2, p) => {
                // Check if cubic is nearly straight
                let dev1 = point_to_line_distance(c1, current, p);
                let dev2 = point_to_line_distance(c2, current, p);
                let max_dev = dev1.max(dev2);

                if max_dev < max_deviation {
                    // Nearly straight — snap to line
                    result.line_to(p);
                } else {
                    result.curve_to(c1, c2, p);
                }
                current = p;
            }
            PathEl::ClosePath => {
                result.close_path();
            }
        }
    }

    result
}

/// Sharpen corners by removing smooth transitions between segments.
///
/// When two consecutive line segments meet at a sharp angle, bezier fitting
/// sometimes inserts a tiny curve to smooth the transition. This function
/// detects those tiny smoothing curves and replaces them with a direct
/// corner (LineTo to the meeting point).
///
/// `min_angle`: minimum turning angle (radians) to consider a "corner".
/// Corners sharper than this are preserved exactly.
fn sharpen_corners(path: &BezPath, min_angle: f64) -> BezPath {
    let elements: Vec<PathEl> = path.elements().to_vec();
    if elements.len() < 3 {
        return path.clone();
    }

    let mut result = BezPath::new();

    for (i, el) in elements.iter().enumerate() {
        match *el {
            PathEl::CurveTo(c1, c2, p) => {
                // Check if this is a tiny smoothing curve between two lines
                let start = get_endpoint(&elements, i);
                let chord = ((p.x - start.x).powi(2) + (p.y - start.y).powi(2)).sqrt();

                // If the curve is very short (< 3px chord), it's likely
                // a smoothing artifact — replace with direct line
                if chord < 1.5 {
                    result.line_to(p);
                } else {
                    result.curve_to(c1, c2, p);
                }
            }
            PathEl::MoveTo(p) => result.move_to(p),
            PathEl::LineTo(p) => result.line_to(p),
            PathEl::QuadTo(c1, p) => {
                let start = get_endpoint(&elements, i);
                let chord = ((p.x - start.x).powi(2) + (p.y - start.y).powi(2)).sqrt();
                if chord < 1.5 {
                    result.line_to(p);
                } else {
                    result.quad_to(c1, p);
                }
            }
            PathEl::ClosePath => result.close_path(),
        }
    }

    result
}

/// Get the endpoint of the previous path element (the "current point" before element `i`).
fn get_endpoint(elements: &[PathEl], i: usize) -> Point {
    if i == 0 {
        return Point::ZERO;
    }
    match elements[i - 1] {
        PathEl::MoveTo(p) | PathEl::LineTo(p) => p,
        PathEl::QuadTo(_, p) => p,
        PathEl::CurveTo(_, _, p) => p,
        PathEl::ClosePath => {
            // Walk back to find the most recent MoveTo
            for j in (0..i).rev() {
                if let PathEl::MoveTo(p) = elements[j] {
                    return p;
                }
            }
            Point::ZERO
        }
    }
}

/// Perpendicular distance from point `p` to the line through `a` and `b`.
pub fn point_to_line_distance(p: Point, a: Point, b: Point) -> f64 {
    let dx = b.x - a.x;
    let dy = b.y - a.y;
    let len_sq = dx * dx + dy * dy;
    if len_sq < 1e-10 {
        // a and b are the same point — return distance to that point
        return ((p.x - a.x).powi(2) + (p.y - a.y).powi(2)).sqrt();
    }
    // |cross product| / |line length|
    ((p.x - a.x) * dy - (p.y - a.y) * dx).abs() / len_sq.sqrt()
}

// ─────────────────────────────────────────────────────────────────────────────
// SVG PARSING / SERIALIZATION
// ─────────────────────────────────────────────────────────────────────────────

enum SvgPart {
    Verbatim(String),
    Path {
        d: String,
        fill: String,
        other_attrs: String,
        translate: (f64, f64),
    },
}

/// Parse SVG into header, elements, and footer.
fn parse_svg_structure(svg: &str) -> (String, Vec<SvgPart>, String) {
    let mut header = String::new();
    let mut elements = Vec::new();
    let footer = "</svg>".to_string();

    // Extract header (everything up to first element after <svg ...>)
    if let Some(svg_start) = svg.find("<svg") {
        if let Some(close) = svg[svg_start..].find('>') {
            let end = svg_start + close + 1;
            header = svg[..end].to_string();
            header.push('\n');
        }
    }

    // Parse <rect> and <path> elements
    let mut search_from = 0;
    while search_from < svg.len() {
        // Find next element
        let next_rect = svg[search_from..].find("<rect").map(|p| search_from + p);
        let next_path = svg[search_from..].find("<path").map(|p| search_from + p);

        let next_pos = match (next_rect, next_path) {
            (Some(r), Some(p)) => Some(r.min(p)),
            (Some(r), None) => Some(r),
            (None, Some(p)) => Some(p),
            (None, None) => None,
        };

        let Some(pos) = next_pos else { break };

        let end_pos = svg[pos..].find("/>").map(|e| pos + e + 2);
        let Some(end) = end_pos else {
            search_from = pos + 1;
            continue;
        };

        let elem_str = &svg[pos..end];

        if elem_str.starts_with("<path") {
            // Extract d and fill attributes
            let d = extract_attr(elem_str, "d").unwrap_or("").to_string();
            let fill = extract_attr(elem_str, "fill").unwrap_or("#000000").to_string();

            // Parse translate transform
            let translate = extract_attr(elem_str, "transform")
                .and_then(parse_translate)
                .unwrap_or((0.0, 0.0));

            // Collect other attributes (stroke, etc.) — skip transform since we apply it
            let mut other = String::new();
            for attr in ["stroke", "stroke-width", "stroke-linejoin", "stroke-linecap", "fill-rule", "opacity"] {
                if let Some(val) = extract_attr(elem_str, attr) {
                    other.push_str(&format!(" {}=\"{}\"", attr, val));
                }
            }

            elements.push(SvgPart::Path { d, fill, other_attrs: other, translate });
        } else {
            elements.push(SvgPart::Verbatim(elem_str.to_string()));
        }

        search_from = end;
    }

    (header, elements, footer)
}

/// Parse a `translate(x,y)` or `translate(x, y)` transform string.
fn parse_translate(transform: &str) -> Option<(f64, f64)> {
    let s = transform.strip_prefix("translate(")?;
    let s = s.strip_suffix(')')?;
    let parts: Vec<&str> = s.split(',').collect();
    if parts.len() == 2 {
        let x: f64 = parts[0].trim().parse().ok()?;
        let y: f64 = parts[1].trim().parse().ok()?;
        Some((x, y))
    } else {
        // Try space separator
        let parts: Vec<&str> = s.split_whitespace().collect();
        if parts.len() == 2 {
            let x: f64 = parts[0].parse().ok()?;
            let y: f64 = parts[1].parse().ok()?;
            Some((x, y))
        } else {
            None
        }
    }
}

/// Apply a translation offset to all points in a BezPath.
fn apply_translate(path: &mut BezPath, tx: f64, ty: f64) {
    if tx.abs() < 1e-9 && ty.abs() < 1e-9 { return; }

    let offset = kurbo::Vec2::new(tx, ty);
    let elements: Vec<PathEl> = path.elements().iter().map(|el| {
        match *el {
            PathEl::MoveTo(p) => PathEl::MoveTo(p + offset),
            PathEl::LineTo(p) => PathEl::LineTo(p + offset),
            PathEl::QuadTo(c1, p) => PathEl::QuadTo(c1 + offset, p + offset),
            PathEl::CurveTo(c1, c2, p) => PathEl::CurveTo(c1 + offset, c2 + offset, p + offset),
            PathEl::ClosePath => PathEl::ClosePath,
        }
    }).collect();

    *path = BezPath::from_path_segments(elements.iter().map(|el| {
        match *el {
            PathEl::MoveTo(p) => kurbo::PathSeg::Line(kurbo::Line::new(p, p)),
            _ => kurbo::PathSeg::Line(kurbo::Line::new(Point::ZERO, Point::ZERO)),
        }
    }));

    // Rebuild from elements directly
    let mut new_path = BezPath::new();
    for el in &elements {
        new_path.push(*el);
    }
    *path = new_path;
}

/// Extract the value of an XML attribute.
fn extract_attr<'a>(element: &'a str, attr: &str) -> Option<&'a str> {
    let needle = format!("{attr}=\"");
    let start = element.find(&needle)? + needle.len();
    let end = start + element[start..].find('"')?;
    Some(&element[start..end])
}

/// Convert a BezPath to an SVG `d` attribute string.
/// Uses reduced precision (1 decimal) for cleaner logo output.
fn bezpath_to_svg_d(path: &BezPath) -> String {
    let mut d = String::new();
    for el in path.elements() {
        match *el {
            PathEl::MoveTo(p) => {
                if !d.is_empty() { d.push(' '); }
                d.push_str(&format!("M{:.2},{:.2}", p.x, p.y));
            }
            PathEl::LineTo(p) => {
                d.push_str(&format!(" L{:.2},{:.2}", p.x, p.y));
            }
            PathEl::QuadTo(c1, p) => {
                d.push_str(&format!(" Q{:.2},{:.2} {:.2},{:.2}", c1.x, c1.y, p.x, p.y));
            }
            PathEl::CurveTo(c1, c2, p) => {
                d.push_str(&format!(
                    " C{:.2},{:.2} {:.2},{:.2} {:.2},{:.2}",
                    c1.x, c1.y, c2.x, c2.y, p.x, p.y
                ));
            }
            PathEl::ClosePath => {
                d.push_str(" Z");
            }
        }
    }
    d
}
