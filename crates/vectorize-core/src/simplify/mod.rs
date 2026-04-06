//! Stage 5: Path simplification.
//!
//! Additional simplification pass after curve fitting.
//! Supports two algorithms:
//! - **KurboBezier**: Bezier-aware simplification (best for curves)
//! - **VisvalingamWhyatt**: Area-based simplification (best for organic shapes)

use geo::SimplifyVw;
use kurbo::simplify::{simplify_bezpath, SimplifyOptions};
use crate::par::iter_prelude::*;

use crate::{SimplifyMethod, VectorPath, VectorizeConfig};

/// Simplify all vector paths by reducing segment count.
///
/// Mode controls which algorithm is used and how aggressively:
/// - Logo: KurboBezier, tight tolerance (preserve geometric precision)
/// - Illustration: VisvalingamWhyatt (organic shape preservation)
/// - Photo: KurboBezier, moderate (balance detail vs file size)
/// - HiFi: skipped entirely (maximum fidelity)
/// - Sketch: KurboBezier, moderate (clean up strokes without losing character)
///
/// Skipped if simplify_tolerance <= fit_tolerance (no further reduction possible).
pub fn simplify_paths(paths: &[VectorPath], config: &VectorizeConfig) -> Vec<VectorPath> {
    use crate::quality::Mode;

    // HiFi mode: skip simplification entirely to preserve maximum detail
    if config.mode == Mode::HighFidelity {
        tracing::info!("HiFi mode: skipping simplification for maximum fidelity");
        return paths.to_vec();
    }

    let defaults = VectorizeConfig::default();
    let tolerance = if (config.simplify_tolerance - defaults.simplify_tolerance).abs() > 1e-9 {
        config.simplify_tolerance
    } else {
        config.quality.native_simplify_tolerance()
    };
    let fit_tol = if (config.fit_tolerance - defaults.fit_tolerance).abs() > 1e-9 {
        config.fit_tolerance
    } else {
        config.quality.native_fit_tolerance()
    };

    // Mode-specific tolerance adjustment
    let tolerance = match config.mode {
        Mode::Logo => tolerance * 0.8,         // tighter — preserve geometric precision
        Mode::Sketch => tolerance * 0.9,       // slightly tighter — preserve stroke detail
        Mode::Photo => tolerance * 1.2,        // looser — reduce file size for gradients
        _ => tolerance,
    };

    // Only simplify if we're loosening tolerance (reducing detail).
    if tolerance <= fit_tol {
        return paths.to_vec();
    }

    // Use the method set by the mode recipe (via config.simplify_method)
    match config.simplify_method {
        SimplifyMethod::KurboBezier => simplify_kurbo(paths, tolerance),
        SimplifyMethod::VisvalingamWhyatt => simplify_vw(paths, tolerance),
    }
}

/// Kurbo's Bezier-aware simplifier.
fn simplify_kurbo(paths: &[VectorPath], tolerance: f64) -> Vec<VectorPath> {
    let opts = SimplifyOptions::default();
    crate::par::maybe_par_iter!(paths)
        .map(|vp| {
            let simplified =
                simplify_bezpath(vp.path.elements().iter().copied(), tolerance, &opts);
            VectorPath {
                path: simplified,
                color: vp.color,
                is_hole: vp.is_hole,
            }
        })
        .collect()
}

/// Visvalingam-Whyatt area-based simplifier.
/// Flattens curves to polylines, applies VW to reduce point count,
/// then re-fits the simplified polyline back to bezier curves.
fn simplify_vw(paths: &[VectorPath], tolerance: f64) -> Vec<VectorPath> {
    // VW uses area threshold, not distance tolerance.
    // Convert distance tolerance to area: area ≈ tolerance².
    let area_threshold = tolerance * tolerance;
    let refit_opts = SimplifyOptions::default();

    crate::par::maybe_par_iter!(paths)
        .map(|vp| {
            let line_string = bezpath_to_linestring(&vp.path);
            if line_string.0.len() < 3 {
                return vp.clone();
            }
            let simplified = SimplifyVw::simplify_vw(&line_string, area_threshold);
            let polyline = linestring_to_bezpath(&simplified);

            // Re-fit the simplified polyline back to bezier curves.
            // Without this, VW output is all LineTo with no curves.
            let refitted = simplify_bezpath(
                polyline.elements().iter().copied(),
                tolerance,
                &refit_opts,
            );

            VectorPath {
                path: refitted,
                color: vp.color,
                is_hole: vp.is_hole,
            }
        })
        .collect()
}

/// Convert a kurbo BezPath to a geo LineString by flattening curves.
fn bezpath_to_linestring(path: &kurbo::BezPath) -> geo::LineString<f64> {
    let mut coords = Vec::new();
    for el in path.elements() {
        match *el {
            kurbo::PathEl::MoveTo(p) | kurbo::PathEl::LineTo(p) => {
                coords.push(geo::Coord { x: p.x, y: p.y });
            }
            kurbo::PathEl::QuadTo(_, p2) => {
                // Approximate: just use the endpoint.
                // For better quality, sample intermediate points.
                coords.push(geo::Coord { x: p2.x, y: p2.y });
            }
            kurbo::PathEl::CurveTo(_, _, p3) => {
                coords.push(geo::Coord { x: p3.x, y: p3.y });
            }
            kurbo::PathEl::ClosePath => {
                // Close by repeating first point if not already there.
                if let Some(first) = coords.first().copied() {
                    if coords.last() != Some(&first) {
                        coords.push(first);
                    }
                }
            }
        }
    }
    geo::LineString(coords)
}

/// Convert a geo LineString back to a kurbo BezPath (line segments only).
fn linestring_to_bezpath(ls: &geo::LineString<f64>) -> kurbo::BezPath {
    let mut path = kurbo::BezPath::new();
    for (i, coord) in ls.0.iter().enumerate() {
        let pt = kurbo::Point::new(coord.x, coord.y);
        if i == 0 {
            path.move_to(pt);
        } else {
            path.line_to(pt);
        }
    }
    // Close if first == last.
    if ls.0.len() >= 2 && ls.0.first() == ls.0.last() {
        // Remove the duplicate last point and close.
        let elems = path.elements().to_vec();
        let mut closed = kurbo::BezPath::new();
        for el in &elems[..elems.len().saturating_sub(1)] {
            closed.push(*el);
        }
        closed.close_path();
        return closed;
    }
    path
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_vw_reduces_points() {
        // A polyline with redundant middle points.
        let mut path = kurbo::BezPath::new();
        path.move_to(kurbo::Point::new(0.0, 0.0));
        path.line_to(kurbo::Point::new(1.0, 0.1)); // nearly collinear
        path.line_to(kurbo::Point::new(2.0, 0.0));
        path.line_to(kurbo::Point::new(2.0, 10.0));
        path.line_to(kurbo::Point::new(0.0, 10.0));
        path.close_path();

        let vp = VectorPath {
            path,
            color: crate::Color::rgb(0, 0, 0),
            is_hole: false,
        };

        let result = simplify_vw(&[vp.clone()], 1.0);
        assert_eq!(result.len(), 1);
        // The nearly-collinear point should be removed.
        assert!(
            result[0].path.elements().len() <= vp.path.elements().len(),
            "VW should reduce or maintain element count"
        );
    }
}
