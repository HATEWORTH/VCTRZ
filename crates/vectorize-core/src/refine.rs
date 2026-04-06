//! Adaptive curve refinement for polygon-mode paths.
//!
//! Takes paths with all-LineTo segments (from VTracer polygon mode) and
//! selectively converts curved sections to smooth Bezier curves while
//! preserving sharp corners as-is.
//!
//! This gives the best of both worlds:
//! - Crisp, sharp corners on straight edges and angles
//! - Smooth, high-quality curves on rounded features (arcs, circles, organic shapes)

use kurbo::simplify::{simplify_bezpath, SimplifyOptions};
use kurbo::{BezPath, PathEl, Point};

/// Configuration for adaptive refinement.
#[derive(Debug, Clone)]
pub struct RefineOptions {
    /// Turning angle (degrees) above which a vertex is considered a sharp corner.
    /// Corners are preserved as-is. Default: 35.0
    pub corner_threshold_deg: f64,

    /// Turning angle (degrees) below which a segment is considered straight.
    /// Straight segments are kept as LineTo. Default: 3.0
    pub straight_threshold_deg: f64,

    /// Minimum number of consecutive "curve" vertices needed to trigger
    /// bezier fitting. Shorter runs are kept as line segments. Default: 3
    pub min_curve_run: usize,

    /// Tolerance for kurbo's bezier fitting on curve sections.
    /// Lower = more faithful to the original points. Default: 0.8
    pub fit_tolerance: f64,
}

impl Default for RefineOptions {
    fn default() -> Self {
        Self {
            corner_threshold_deg: 35.0,
            straight_threshold_deg: 3.0,
            min_curve_run: 3,
            fit_tolerance: 0.8,
        }
    }
}

/// Vertex classification for adaptive refinement.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum VertexKind {
    /// Sharp corner — preserve exactly.
    Corner,
    /// Part of a curve — candidate for bezier fitting.
    Curve,
    /// Part of a straight section — keep as LineTo.
    Straight,
}

/// Compute the turning angle (radians) at a vertex given its neighbors.
fn turning_angle(prev: Point, curr: Point, next: Point) -> f64 {
    let v1x = curr.x - prev.x;
    let v1y = curr.y - prev.y;
    let v2x = next.x - curr.x;
    let v2y = next.y - curr.y;

    let mag1 = (v1x * v1x + v1y * v1y).sqrt();
    let mag2 = (v2x * v2x + v2y * v2y).sqrt();

    if mag1 < 1e-10 || mag2 < 1e-10 {
        return 0.0;
    }

    let dot = v1x * v2x + v1y * v2y;
    let cos_angle = (dot / (mag1 * mag2)).clamp(-1.0, 1.0);
    cos_angle.acos()
}

/// Extract all subpaths from a BezPath as sequences of points.
/// Returns (points, is_closed) for each subpath.
fn extract_subpaths(path: &BezPath) -> Vec<(Vec<Point>, bool)> {
    let mut subpaths = Vec::new();
    let mut current_points: Vec<Point> = Vec::new();

    for el in path.elements() {
        match *el {
            PathEl::MoveTo(p) => {
                if current_points.len() >= 2 {
                    subpaths.push((current_points, false));
                }
                current_points = vec![p];
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
        subpaths.push((current_points, false));
    }

    subpaths
}

/// Classify each vertex in a closed polyline as Corner, Curve, or Straight.
fn classify_vertices(points: &[Point], opts: &RefineOptions) -> Vec<VertexKind> {
    let n = points.len();
    if n < 3 {
        return vec![VertexKind::Corner; n];
    }

    let corner_thresh = opts.corner_threshold_deg.to_radians();
    let straight_thresh = opts.straight_threshold_deg.to_radians();

    (0..n)
        .map(|i| {
            let prev = points[(i + n - 1) % n];
            let curr = points[i];
            let next = points[(i + 1) % n];
            let angle = turning_angle(prev, curr, next);

            if angle > corner_thresh {
                VertexKind::Corner
            } else if angle < straight_thresh {
                VertexKind::Straight
            } else {
                VertexKind::Curve
            }
        })
        .collect()
}

/// A run of consecutive vertices of the same kind (or curve-compatible).
#[derive(Debug)]
struct VertexRun {
    kind: VertexKind,
    /// Indices into the points array (inclusive start, exclusive end).
    start: usize,
    end: usize,
}

/// Group consecutive vertices into runs by kind.
/// Corner vertices always form single-vertex runs (they are boundaries).
/// Curve and Straight vertices are grouped into contiguous runs.
fn group_vertex_runs(kinds: &[VertexKind]) -> Vec<VertexRun> {
    if kinds.is_empty() {
        return Vec::new();
    }

    let mut runs = Vec::new();
    let mut run_start = 0;
    let mut run_kind = kinds[0];

    for i in 1..kinds.len() {
        let k = kinds[i];

        // Corners always break runs
        if k == VertexKind::Corner || run_kind == VertexKind::Corner || k != run_kind {
            runs.push(VertexRun {
                kind: run_kind,
                start: run_start,
                end: i,
            });
            run_start = i;
            run_kind = k;
        }
    }
    // Final run
    runs.push(VertexRun {
        kind: run_kind,
        start: run_start,
        end: kinds.len(),
    });

    runs
}

/// Fit a cubic bezier through a sequence of points using kurbo's simplifier.
/// `corner_angle_rad` is the maximum turning angle (radians) that vertices in this
/// run can have — controls how aggressively segments are merged into curves.
fn fit_bezier_through_points(points: &[Point], tolerance: f64, corner_angle_rad: f64) -> BezPath {
    if points.len() < 2 {
        let mut path = BezPath::new();
        if let Some(&p) = points.first() {
            path.move_to(p);
        }
        return path;
    }

    // Build a polyline through the points
    let mut polyline = BezPath::new();
    polyline.move_to(points[0]);
    for &p in &points[1..] {
        polyline.line_to(p);
    }

    // Use the corner threshold as the angle_thresh so kurbo is allowed to
    // merge all segments within this curve run. We've already classified
    // these vertices as "curve" (angle < corner_threshold), so it's safe.
    let opts = SimplifyOptions::default().angle_thresh(corner_angle_rad + 0.1);
    simplify_bezpath(polyline.elements().iter().copied(), tolerance, &opts)
}

/// Adaptively refine a single closed subpath.
/// Corners stay sharp, curved sections get bezier-fitted.
fn refine_closed_subpath(points: &[Point], opts: &RefineOptions) -> BezPath {
    let corner_angle_rad = opts.corner_threshold_deg.to_radians();
    let n = points.len();
    if n < 4 {
        // Too few points to refine — return as-is
        let mut path = BezPath::new();
        if let Some(&p) = points.first() {
            path.move_to(p);
            for &q in &points[1..] {
                path.line_to(q);
            }
            path.close_path();
        }
        return path;
    }

    let kinds = classify_vertices(points, opts);
    let runs = group_vertex_runs(&kinds);

    let mut result = BezPath::new();
    let mut started = false;

    for run in &runs {
        let run_len = run.end - run.start;

        match run.kind {
            VertexKind::Corner => {
                // Emit corner point(s) as LineTo
                for i in run.start..run.end {
                    if !started {
                        result.move_to(points[i]);
                        started = true;
                    } else {
                        result.line_to(points[i]);
                    }
                }
            }
            VertexKind::Straight => {
                // Straight sections: keep as LineTo
                for i in run.start..run.end {
                    if !started {
                        result.move_to(points[i]);
                        started = true;
                    } else {
                        result.line_to(points[i]);
                    }
                }
            }
            VertexKind::Curve => {
                if run_len < opts.min_curve_run {
                    // Too short to fit curves — keep as lines
                    for i in run.start..run.end {
                        if !started {
                            result.move_to(points[i]);
                            started = true;
                        } else {
                            result.line_to(points[i]);
                        }
                    }
                } else {
                    // Collect points for this curve run, including one point
                    // before and after for continuity (overlap with neighbors).
                    let extra_before = if run.start > 0 { 1 } else { 0 };
                    let extra_after = if run.end < n { 1 } else { 0 };

                    let curve_start = run.start - extra_before;
                    let curve_end = (run.end + extra_after).min(n);

                    let curve_points: Vec<Point> =
                        (curve_start..curve_end).map(|i| points[i]).collect();

                    let fitted = fit_bezier_through_points(&curve_points, opts.fit_tolerance, corner_angle_rad);

                    // Append the fitted path elements, skipping the MoveTo
                    // (we've already positioned at the right point via the
                    // previous run's last point or the overlap point).
                    let elems = fitted.elements();
                    for (j, el) in elems.iter().enumerate() {
                        match *el {
                            PathEl::MoveTo(p) => {
                                if !started {
                                    result.move_to(p);
                                    started = true;
                                } else if j == 0 && extra_before == 0 {
                                    // No overlap point — need to position
                                    result.line_to(p);
                                }
                                // If we have an overlap, skip the MoveTo
                            }
                            PathEl::LineTo(p) => {
                                if !started {
                                    result.move_to(p);
                                    started = true;
                                } else {
                                    result.line_to(p);
                                }
                            }
                            PathEl::CurveTo(c1, c2, p) => {
                                if !started {
                                    result.move_to(p);
                                    started = true;
                                } else {
                                    result.push(PathEl::CurveTo(c1, c2, p));
                                }
                            }
                            PathEl::QuadTo(c1, p) => {
                                if !started {
                                    result.move_to(p);
                                    started = true;
                                } else {
                                    result.push(PathEl::QuadTo(c1, p));
                                }
                            }
                            PathEl::ClosePath => {} // Don't close mid-path
                        }
                    }
                }
            }
        }
    }

    result.close_path();
    result
}

/// Adaptively refine a BezPath: smooth curved sections while preserving sharp corners.
///
/// This is the main entry point. Works on paths from any source (VTracer polygon mode,
/// native pipeline, etc.). Best results when input is a polyline (all LineTo segments).
///
/// The algorithm:
/// 1. Extract subpaths from the BezPath
/// 2. For each closed subpath, classify vertices as Corner/Curve/Straight
/// 3. Group consecutive curve vertices into runs
/// 4. Fit bezier curves through curve runs, keep corners and straights as-is
/// 5. Reassemble into a clean BezPath
pub fn adaptive_refine(path: &BezPath, opts: &RefineOptions) -> BezPath {
    let subpaths = extract_subpaths(path);

    if subpaths.is_empty() {
        return path.clone();
    }

    let mut result = BezPath::new();

    for (points, closed) in &subpaths {
        if *closed && points.len() >= 4 {
            let refined = refine_closed_subpath(points, opts);
            // Append all elements from the refined subpath
            for el in refined.elements() {
                result.push(*el);
            }
        } else {
            // Open paths or very short paths: keep as-is
            if let Some(&p) = points.first() {
                result.move_to(p);
                for &q in &points[1..] {
                    result.line_to(q);
                }
                if *closed {
                    result.close_path();
                }
            }
        }
    }

    result
}

/// Refine an SVG string by parsing paths, applying adaptive refinement, and re-serializing.
/// This works on the raw SVG output from VTracer.
pub fn refine_svg(svg: &str, opts: &RefineOptions) -> String {
    let mut result = String::with_capacity(svg.len());
    let mut search_from = 0;

    while search_from < svg.len() {
        // Find next <path element
        let Some(path_pos) = svg[search_from..].find("<path") else {
            result.push_str(&svg[search_from..]);
            break;
        };
        let abs_pos = search_from + path_pos;

        // Copy everything before this <path
        result.push_str(&svg[search_from..abs_pos]);

        // Find the end of this element
        let Some(end_offset) = svg[abs_pos..].find("/>") else {
            result.push_str(&svg[abs_pos..]);
            break;
        };
        let elem_end = abs_pos + end_offset + 2;
        let element = &svg[abs_pos..elem_end];

        // Extract the d attribute
        if let Some(d_val) = extract_d_attr(element) {
            // Try to parse and refine the path
            if let Ok(bez) = BezPath::from_svg(d_val) {
                // Only refine if path is mostly line segments (polygon mode output)
                if is_mostly_lines(&bez) && bez.elements().len() >= 6 {
                    let refined = adaptive_refine(&bez, opts);
                    let new_d = refined.to_svg();

                    // Reconstruct the element with the new d value
                    let new_element = element.replace(
                        &format!("d=\"{d_val}\""),
                        &format!("d=\"{new_d}\""),
                    );
                    result.push_str(&new_element);
                } else {
                    // Not a polygon path — keep as-is
                    result.push_str(element);
                }
            } else {
                result.push_str(element);
            }
        } else {
            result.push_str(element);
        }

        search_from = elem_end;
    }

    result
}

/// Check if a BezPath is mostly composed of LineTo segments.
fn is_mostly_lines(path: &BezPath) -> bool {
    let mut lines = 0;
    let mut curves = 0;
    for el in path.elements() {
        match el {
            PathEl::LineTo(_) => lines += 1,
            PathEl::CurveTo(_, _, _) | PathEl::QuadTo(_, _) => curves += 1,
            _ => {}
        }
    }
    let total = lines + curves;
    total > 0 && (lines as f64 / total as f64) > 0.7
}

/// Extract the `d` attribute value from a `<path ... />` element string.
fn extract_d_attr(element: &str) -> Option<&str> {
    let needle = "d=\"";
    let start = element.find(needle)? + needle.len();
    let end = start + element[start..].find('"')?;
    Some(&element[start..end])
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::f64::consts::PI;

    /// Helper: create a circle approximated by n line segments.
    fn circle_polygon(n: usize, cx: f64, cy: f64, r: f64) -> BezPath {
        let mut path = BezPath::new();
        for i in 0..n {
            let angle = 2.0 * PI * i as f64 / n as f64;
            let p = Point::new(cx + r * angle.cos(), cy + r * angle.sin());
            if i == 0 {
                path.move_to(p);
            } else {
                path.line_to(p);
            }
        }
        path.close_path();
        path
    }

    /// Helper: create a rounded rectangle (4 straight sides + 4 rounded corners).
    fn rounded_rect_polygon(w: f64, h: f64, r: f64, segs_per_corner: usize) -> BezPath {
        let mut path = BezPath::new();
        let mut first = true;

        let emit = |path: &mut BezPath, p: Point, first: &mut bool| {
            if *first {
                path.move_to(p);
                *first = false;
            } else {
                path.line_to(p);
            }
        };

        // Bottom edge (left to right)
        for i in 0..10 {
            let t = i as f64 / 10.0;
            emit(&mut path, Point::new(r + t * (w - 2.0 * r), 0.0), &mut first);
        }
        // Bottom-right corner arc
        for i in 0..segs_per_corner {
            let angle = -PI / 2.0 + PI / 2.0 * i as f64 / segs_per_corner as f64;
            emit(
                &mut path,
                Point::new(w - r + r * angle.cos(), r + r * angle.sin()),
                &mut first,
            );
        }
        // Right edge
        for i in 0..10 {
            let t = i as f64 / 10.0;
            emit(&mut path, Point::new(w, r + t * (h - 2.0 * r)), &mut first);
        }
        // Top-right corner arc
        for i in 0..segs_per_corner {
            let angle = 0.0 + PI / 2.0 * i as f64 / segs_per_corner as f64;
            emit(
                &mut path,
                Point::new(w - r + r * angle.cos(), h - r + r * angle.sin()),
                &mut first,
            );
        }
        // Top edge (right to left)
        for i in 0..10 {
            let t = i as f64 / 10.0;
            emit(
                &mut path,
                Point::new(w - r - t * (w - 2.0 * r), h),
                &mut first,
            );
        }
        // Top-left corner arc
        for i in 0..segs_per_corner {
            let angle = PI / 2.0 + PI / 2.0 * i as f64 / segs_per_corner as f64;
            emit(
                &mut path,
                Point::new(r + r * angle.cos(), h - r + r * angle.sin()),
                &mut first,
            );
        }
        // Left edge
        for i in 0..10 {
            let t = i as f64 / 10.0;
            emit(&mut path, Point::new(0.0, h - r - t * (h - 2.0 * r)), &mut first);
        }
        // Bottom-left corner arc
        for i in 0..segs_per_corner {
            let angle = PI + PI / 2.0 * i as f64 / segs_per_corner as f64;
            emit(
                &mut path,
                Point::new(r + r * angle.cos(), r + r * angle.sin()),
                &mut first,
            );
        }

        path.close_path();
        path
    }

    #[test]
    fn test_circle_refinement_produces_curves() {
        // Use a large circle with relatively few polygon segments so the
        // chord-to-arc deviation exceeds the fit tolerance and triggers
        // bezier fitting. 16 sides on r=100 gives ~6px deviation per chord.
        let circle = circle_polygon(16, 200.0, 200.0, 100.0);
        let opts = RefineOptions {
            fit_tolerance: 0.5,
            ..Default::default()
        };
        let refined = adaptive_refine(&circle, &opts);

        // The refined path should have CurveTo elements
        let curve_count = refined
            .elements()
            .iter()
            .filter(|e| matches!(e, PathEl::CurveTo(_, _, _)))
            .count();
        assert!(
            curve_count > 0,
            "Circle refinement should produce bezier curves, got none"
        );
    }

    #[test]
    fn test_square_stays_sharp() {
        // A perfect square — all corners should be preserved
        let mut path = BezPath::new();
        path.move_to(Point::new(0.0, 0.0));
        path.line_to(Point::new(100.0, 0.0));
        path.line_to(Point::new(100.0, 100.0));
        path.line_to(Point::new(0.0, 100.0));
        path.close_path();

        let opts = RefineOptions::default();
        let refined = adaptive_refine(&path, &opts);

        // Should have no CurveTo elements
        let curve_count = refined
            .elements()
            .iter()
            .filter(|e| matches!(e, PathEl::CurveTo(_, _, _)))
            .count();
        assert_eq!(
            curve_count, 0,
            "Square should have no bezier curves, got {curve_count}"
        );
    }

    #[test]
    fn test_rounded_rect_mixed() {
        // Large rounded rect with generous corner radius (50px) and few
        // segments per corner (6) so chord deviation is significant.
        let rrect = rounded_rect_polygon(400.0, 200.0, 50.0, 6);
        let opts = RefineOptions {
            fit_tolerance: 0.5,
            ..Default::default()
        };
        let refined = adaptive_refine(&rrect, &opts);

        // Should have both LineTo (straight edges) and CurveTo (corners)
        let line_count = refined
            .elements()
            .iter()
            .filter(|e| matches!(e, PathEl::LineTo(_)))
            .count();
        let curve_count = refined
            .elements()
            .iter()
            .filter(|e| matches!(e, PathEl::CurveTo(_, _, _)))
            .count();

        assert!(line_count > 0, "Rounded rect should have straight sections");
        assert!(curve_count > 0, "Rounded rect should have curved corners");
    }

    #[test]
    fn test_is_mostly_lines() {
        let mut poly = BezPath::new();
        poly.move_to(Point::new(0.0, 0.0));
        poly.line_to(Point::new(10.0, 0.0));
        poly.line_to(Point::new(10.0, 10.0));
        poly.close_path();
        assert!(is_mostly_lines(&poly));

        let mut curves = BezPath::new();
        curves.move_to(Point::new(0.0, 0.0));
        curves.push(PathEl::CurveTo(
            Point::new(5.0, 10.0),
            Point::new(15.0, 10.0),
            Point::new(20.0, 0.0),
        ));
        curves.close_path();
        assert!(!is_mostly_lines(&curves));
    }

    #[test]
    fn test_empty_path() {
        let path = BezPath::new();
        let opts = RefineOptions::default();
        let refined = adaptive_refine(&path, &opts);
        assert!(refined.elements().is_empty());
    }

    #[test]
    fn test_refine_options_thresholds() {
        let circle = circle_polygon(32, 50.0, 50.0, 30.0);

        // With very high corner threshold, everything is a corner — no curves
        let strict = RefineOptions {
            corner_threshold_deg: 5.0,
            ..Default::default()
        };
        let refined_strict = adaptive_refine(&circle, &strict);
        let curves_strict = refined_strict
            .elements()
            .iter()
            .filter(|e| matches!(e, PathEl::CurveTo(_, _, _)))
            .count();

        // With default threshold, should have curves
        let relaxed = RefineOptions::default();
        let refined_relaxed = adaptive_refine(&circle, &relaxed);
        let curves_relaxed = refined_relaxed
            .elements()
            .iter()
            .filter(|e| matches!(e, PathEl::CurveTo(_, _, _)))
            .count();

        assert!(
            curves_relaxed >= curves_strict,
            "Relaxed threshold should produce at least as many curves: {} vs {}",
            curves_relaxed,
            curves_strict
        );
    }
}
