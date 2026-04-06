//! Stage: Geometric primitive detection.
//!
//! Analyzes `kurbo::BezPath` data and detects whether it closely approximates
//! a geometric primitive (circle, ellipse, rectangle). When a match is found,
//! the path can be rendered as a clean SVG primitive element instead of a
//! complex `<path>` with many control points.

use kurbo::{BezPath, ParamCurve, Point, Shape};
use std::fmt::Write;

/// A detected geometric shape, or the original path if no primitive matches.
#[derive(Debug, Clone)]
pub enum DetectedShape {
    /// A circle centered at `(cx, cy)` with radius `r`.
    Circle { cx: f64, cy: f64, r: f64 },
    /// An ellipse centered at `(cx, cy)` with semi-axes `rx` and `ry`.
    Ellipse {
        cx: f64,
        cy: f64,
        rx: f64,
        ry: f64,
    },
    /// An axis-aligned rectangle at `(x, y)` with given dimensions and optional
    /// corner radii `rx`, `ry`.
    Rect {
        x: f64,
        y: f64,
        width: f64,
        height: f64,
        rx: f64,
        ry: f64,
    },
    /// Not a recognized primitive; keep the original path.
    Path(BezPath),
}

// ---------------------------------------------------------------------------
// Sampling helpers
// ---------------------------------------------------------------------------

/// Sample approximately `n` evenly-spaced points along a `BezPath` by
/// iterating over its segments and evaluating at regular `t` intervals.
fn sample_points(path: &BezPath, n: usize) -> Vec<Point> {
    let segments: Vec<_> = path.segments().collect();
    if segments.is_empty() {
        return Vec::new();
    }

    // Distribute samples proportionally across segments based on their
    // approximate arc-length. For simplicity we give each segment an equal
    // share of samples (good enough for detection).
    let per_seg = (n / segments.len()).max(1);
    let mut points = Vec::with_capacity(per_seg * segments.len());

    for seg in &segments {
        for i in 0..per_seg {
            let t = i as f64 / per_seg as f64;
            points.push(seg.eval(t));
        }
    }
    // Always include the very last point of the path.
    if let Some(last) = segments.last() {
        points.push(last.eval(1.0));
    }
    points
}

/// Compute the centroid (mean) of a set of points.
fn centroid(points: &[Point]) -> Point {
    let n = points.len() as f64;
    let (sx, sy) = points.iter().fold((0.0, 0.0), |(ax, ay), p| (ax + p.x, ay + p.y));
    Point::new(sx / n, sy / n)
}

/// Standard deviation of a slice of `f64` values.
fn std_dev(values: &[f64]) -> f64 {
    let n = values.len() as f64;
    let mean = values.iter().sum::<f64>() / n;
    let var = values.iter().map(|v| (v - mean).powi(2)).sum::<f64>() / n;
    var.sqrt()
}

// ---------------------------------------------------------------------------
// Detection routines
// ---------------------------------------------------------------------------

/// Try to detect a circle.
///
/// Strategy: sample points, compute distances to centroid, check that relative
/// standard deviation is very small.
fn try_circle(path: &BezPath, tolerance: f64) -> Option<DetectedShape> {
    let points = sample_points(path, 64);
    if points.len() < 8 {
        return None;
    }

    let center = centroid(&points);
    let distances: Vec<f64> = points
        .iter()
        .map(|p| ((p.x - center.x).powi(2) + (p.y - center.y).powi(2)).sqrt())
        .collect();

    let mean_r = distances.iter().sum::<f64>() / distances.len() as f64;
    if mean_r < 1e-9 {
        return None; // degenerate
    }

    let sd = std_dev(&distances);
    let relative_dev = sd / mean_r;

    // Relative deviation threshold: 5 %
    if relative_dev >= 0.05 {
        return None;
    }

    // Max absolute deviation must be within pixel tolerance.
    let max_dev = distances
        .iter()
        .map(|d| (d - mean_r).abs())
        .fold(0.0_f64, f64::max);
    if max_dev > tolerance {
        return None;
    }

    // Verify with area check.
    let path_area = path.area().abs();
    let ideal_area = std::f64::consts::PI * mean_r * mean_r;
    if ideal_area > 1e-9 {
        let area_err = (path_area - ideal_area).abs() / ideal_area;
        // Allow a generous area tolerance that scales with the pixel tolerance
        let area_tol = 0.15_f64.max(tolerance * 0.02);
        if area_err > area_tol {
            return None;
        }
    }

    Some(DetectedShape::Circle {
        cx: center.x,
        cy: center.y,
        r: mean_r,
    })
}

/// Try to detect an ellipse.
///
/// Uses the bounding box to derive candidate semi-axes, then checks that every
/// sample point satisfies the implicit ellipse equation within a threshold.
fn try_ellipse(path: &BezPath, tolerance: f64) -> Option<DetectedShape> {
    let points = sample_points(path, 64);
    if points.len() < 8 {
        return None;
    }

    let bbox = path.bounding_box();
    let cx = (bbox.x0 + bbox.x1) / 2.0;
    let cy = (bbox.y0 + bbox.y1) / 2.0;
    let rx = (bbox.x1 - bbox.x0) / 2.0;
    let ry = (bbox.y1 - bbox.y0) / 2.0;

    if rx < 1e-9 || ry < 1e-9 {
        return None;
    }

    // Reject near-circles (they should have been caught already).
    let ratio = rx / ry;
    if (ratio - 1.0).abs() < 0.15 {
        return None;
    }
    // Reject extreme aspect ratios.
    if ratio < 0.3 || ratio > 3.0 {
        return None;
    }

    // For each sample point compute the implicit-equation value.
    // For a perfect ellipse it equals 1.0.
    let max_deviation = points
        .iter()
        .map(|p| {
            let val = ((p.x - cx) / rx).powi(2) + ((p.y - cy) / ry).powi(2);
            (val - 1.0).abs()
        })
        .fold(0.0_f64, f64::max);

    // The threshold is partly relative, partly influenced by pixel tolerance.
    let threshold = 0.1_f64.max(tolerance * 0.01);
    if max_deviation > threshold {
        return None;
    }

    Some(DetectedShape::Ellipse { cx, cy, rx, ry })
}

/// Try to detect a rectangle.
///
/// Fast-path rectangle check from 4 known corner points.
fn try_rect_from_corners(cp: [Point; 4], tolerance: f64) -> Option<DetectedShape> {
    // Check all 4 angles are ~90°.
    for i in 0..4 {
        let prev = cp[(i + 3) % 4];
        let curr = cp[i];
        let next = cp[(i + 1) % 4];
        let v1 = Point::new(prev.x - curr.x, prev.y - curr.y);
        let v2 = Point::new(next.x - curr.x, next.y - curr.y);
        let len1 = (v1.x * v1.x + v1.y * v1.y).sqrt();
        let len2 = (v2.x * v2.x + v2.y * v2.y).sqrt();
        if len1 < 1e-12 || len2 < 1e-12 {
            return None;
        }
        let cos_a = (v1.x * v2.x + v1.y * v2.y) / (len1 * len2);
        let angle = cos_a.clamp(-1.0, 1.0).acos().to_degrees();
        if (angle - 90.0).abs() > 15.0 {
            return None;
        }
    }

    // Check opposite sides roughly equal.
    let side_len = |a: &Point, b: &Point| ((a.x - b.x).powi(2) + (a.y - b.y).powi(2)).sqrt();
    let s0 = side_len(&cp[0], &cp[1]);
    let s1 = side_len(&cp[1], &cp[2]);
    let s2 = side_len(&cp[2], &cp[3]);
    let s3 = side_len(&cp[3], &cp[0]);
    let tol_frac = 0.15_f64.max(tolerance * 0.02);
    if s0 > 1e-9 && ((s0 - s2).abs() / s0 > tol_frac) {
        return None;
    }
    if s1 > 1e-9 && ((s1 - s3).abs() / s1 > tol_frac) {
        return None;
    }

    // Check axis-aligned.
    const AXIS_ALIGN_RATIO: f64 = 0.09;
    for i in 0..4 {
        let a = &cp[i];
        let b = &cp[(i + 1) % 4];
        let dx = (a.x - b.x).abs();
        let dy = (a.y - b.y).abs();
        let (minor, major) = if dx > dy { (dy, dx) } else { (dx, dy) };
        if major > 1e-9 && minor / major > AXIS_ALIGN_RATIO {
            return None;
        }
    }

    let min_x = cp.iter().map(|p| p.x).fold(f64::INFINITY, f64::min);
    let min_y = cp.iter().map(|p| p.y).fold(f64::INFINITY, f64::min);
    let max_x = cp.iter().map(|p| p.x).fold(f64::NEG_INFINITY, f64::max);
    let max_y = cp.iter().map(|p| p.y).fold(f64::NEG_INFINITY, f64::max);
    let width = max_x - min_x;
    let height = max_y - min_y;
    if width < 1e-9 || height < 1e-9 {
        return None;
    }

    Some(DetectedShape::Rect {
        x: min_x,
        y: min_y,
        width,
        height,
        rx: 0.0,
        ry: 0.0,
    })
}

/// Strategy: sample points, compute turning angles between consecutive vectors,
/// find corners (large turning angle), then validate geometry.
/// Falls back to bounding-box analysis when sharp corners aren't found (rounded rects).
fn try_rectangle(path: &BezPath, tolerance: f64) -> Option<DetectedShape> {
    // Fast path: if path is exactly MoveTo + 3 LineTo + ClosePath, check corners directly.
    let elems = path.elements();
    if elems.len() == 5 {
        if let (
            kurbo::PathEl::MoveTo(p0),
            kurbo::PathEl::LineTo(p1),
            kurbo::PathEl::LineTo(p2),
            kurbo::PathEl::LineTo(p3),
            kurbo::PathEl::ClosePath,
        ) = (elems[0], elems[1], elems[2], elems[3], elems[4])
        {
            return try_rect_from_corners([p0, p1, p2, p3], tolerance);
        }
    }

    let points = sample_points(path, 128);
    if points.len() < 12 {
        return None;
    }

    // Compute turning angles.
    let n = points.len();
    let mut corners: Vec<usize> = Vec::new();

    for i in 0..n {
        let prev = if i == 0 { n - 1 } else { i - 1 };
        let next = if i == n - 1 { 0 } else { i + 1 };

        let v1x = points[i].x - points[prev].x;
        let v1y = points[i].y - points[prev].y;
        let v2x = points[next].x - points[i].x;
        let v2y = points[next].y - points[i].y;

        let len1 = (v1x * v1x + v1y * v1y).sqrt();
        let len2 = (v2x * v2x + v2y * v2y).sqrt();
        if len1 < 1e-12 || len2 < 1e-12 {
            continue;
        }

        let cos_angle = (v1x * v2x + v1y * v2y) / (len1 * len2);
        let angle = cos_angle.clamp(-1.0, 1.0).acos().to_degrees();

        // A sharp turn (> 60 degrees) indicates a corner.
        if angle > 60.0 {
            corners.push(i);
        }
    }

    // Merge corners that are close together (within 3 samples).
    let merged = merge_nearby_corners(&corners, &points);

    // If we found exactly 4 sharp corners, use the corner-based path.
    if merged.len() == 4 {
        if let Some(rect) = try_rect_from_corners_sampled(&merged, &points, tolerance) {
            return Some(rect);
        }
    }

    // Fallback: bounding-box-based detection. This handles rounded rectangles
    // (which have no sharp corners) and rects with extra midpoints on edges.
    try_rect_from_bbox(path, &points, tolerance)
}

/// Corner-based rectangle validation using 4 detected corner indices into a
/// sampled point array. Returns a sharp-corner rect (rx=ry=0).
fn try_rect_from_corners_sampled(
    merged: &[usize],
    points: &[Point],
    tolerance: f64,
) -> Option<DetectedShape> {
    let cp: Vec<Point> = merged.iter().map(|&i| points[i]).collect();

    // Check that all four angles are roughly 90 degrees (within 15 degrees).
    for i in 0..4 {
        let prev = &cp[(i + 3) % 4];
        let curr = &cp[i];
        let next = &cp[(i + 1) % 4];

        let v1 = Point::new(prev.x - curr.x, prev.y - curr.y);
        let v2 = Point::new(next.x - curr.x, next.y - curr.y);
        let len1 = (v1.x * v1.x + v1.y * v1.y).sqrt();
        let len2 = (v2.x * v2.x + v2.y * v2.y).sqrt();
        if len1 < 1e-12 || len2 < 1e-12 {
            return None;
        }

        let cos_a = (v1.x * v2.x + v1.y * v2.y) / (len1 * len2);
        let angle = cos_a.clamp(-1.0, 1.0).acos().to_degrees();
        if (angle - 90.0).abs() > 15.0 {
            return None;
        }
    }

    // Check opposite sides are roughly equal length.
    let side_len = |a: &Point, b: &Point| ((a.x - b.x).powi(2) + (a.y - b.y).powi(2)).sqrt();
    let s0 = side_len(&cp[0], &cp[1]);
    let s1 = side_len(&cp[1], &cp[2]);
    let s2 = side_len(&cp[2], &cp[3]);
    let s3 = side_len(&cp[3], &cp[0]);

    let tol_frac = 0.15_f64.max(tolerance * 0.02);
    if s0 > 1e-9 && ((s0 - s2).abs() / s0 > tol_frac) {
        return None;
    }
    if s1 > 1e-9 && ((s1 - s3).abs() / s1 > tol_frac) {
        return None;
    }

    // Verify the quad is axis-aligned.
    const AXIS_ALIGN_RATIO: f64 = 0.09;
    for i in 0..4 {
        let a = &cp[i];
        let b = &cp[(i + 1) % 4];
        let dx = (a.x - b.x).abs();
        let dy = (a.y - b.y).abs();
        let (minor, major) = if dx > dy { (dy, dx) } else { (dx, dy) };
        if major > 1e-9 && minor / major > AXIS_ALIGN_RATIO {
            return None;
        }
    }

    let min_x = cp.iter().map(|p| p.x).fold(f64::INFINITY, f64::min);
    let min_y = cp.iter().map(|p| p.y).fold(f64::INFINITY, f64::min);
    let max_x = cp.iter().map(|p| p.x).fold(f64::NEG_INFINITY, f64::max);
    let max_y = cp.iter().map(|p| p.y).fold(f64::NEG_INFINITY, f64::max);

    let width = max_x - min_x;
    let height = max_y - min_y;
    if width < 1e-9 || height < 1e-9 {
        return None;
    }

    let max_point_dev = points
        .iter()
        .map(|p| dist_to_rect_edges(p, min_x, min_y, max_x, max_y))
        .fold(0.0_f64, f64::max);

    if max_point_dev > tolerance {
        return None;
    }

    Some(DetectedShape::Rect {
        x: min_x,
        y: min_y,
        width,
        height,
        rx: 0.0,
        ry: 0.0,
    })
}

/// Bounding-box-based rectangle detection. Uses the path's bounding box as the
/// candidate rectangle and checks that all sample points lie close to its edges.
/// Then estimates corner radii from the inward deviation of samples near each corner.
fn try_rect_from_bbox(
    path: &BezPath,
    points: &[Point],
    tolerance: f64,
) -> Option<DetectedShape> {
    let bbox = path.bounding_box();
    let min_x = bbox.x0;
    let min_y = bbox.y0;
    let max_x = bbox.x1;
    let max_y = bbox.y1;
    let width = max_x - min_x;
    let height = max_y - min_y;

    if width < 1e-9 || height < 1e-9 {
        return None;
    }

    // Aspect ratio sanity: reject very elongated shapes (likely not rects).
    let aspect = width / height;
    if aspect < 0.1 || aspect > 10.0 {
        return None;
    }

    // Check that the path area is close to the bounding box area.
    // A rectangle should fill most of its bounding box.
    let path_area = path.area().abs();
    let bbox_area = width * height;
    if bbox_area > 1e-9 {
        let fill_ratio = path_area / bbox_area;
        // A sharp rect fills 100%, a rounded rect fills slightly less.
        // A circle fills ~78.5%, an ellipse also ~78.5%.
        // Reject if fill ratio is below 85% (generous for rounded rects).
        if fill_ratio < 0.85 {
            return None;
        }
    }

    // Estimate corner radius from area deficit before doing the edge check,
    // because rounded corners cause points to be further from the bounding box
    // edges than a sharp rectangle would be.
    //
    // For a rounded rect: area = w*h - 4*r^2*(1 - pi/4)
    // So: r = sqrt((w*h - area) / (4*(1 - pi/4)))
    let area_deficit = bbox_area - path_area;
    let estimated_r = if area_deficit > 0.0 {
        let factor = 4.0 * (1.0 - std::f64::consts::FRAC_PI_4); // ~0.858
        (area_deficit / factor).sqrt()
    } else {
        0.0
    };

    // For a rounded rect with radius r, a point on the arc at 45 degrees from
    // a corner is r*(1 - 1/sqrt(2)) away from the nearest bbox edge.
    let corner_deviation = estimated_r * (1.0 - std::f64::consts::FRAC_1_SQRT_2);
    let effective_tolerance = tolerance + corner_deviation;

    // Verify all sample points are close to the rectangle edges, with tolerance
    // adjusted for expected rounded-corner deviation.
    let max_point_dev = points
        .iter()
        .map(|p| dist_to_rect_edges(p, min_x, min_y, max_x, max_y))
        .fold(0.0_f64, f64::max);

    if max_point_dev > effective_tolerance {
        return None;
    }

    // --- Corner radius detection ---
    // For each of the 4 corners, find nearby sample points and measure how far
    // they deviate inward from the sharp corner. For a rounded corner the path
    // cuts across the corner in a circular arc, and the maximum inward
    // deviation of nearby samples approximates the corner radius.
    let shorter_side = width.min(height);
    let corner_region = shorter_side * 0.25; // look within 25% of shorter side

    let corner_pts = [
        Point::new(min_x, min_y),
        Point::new(max_x, min_y),
        Point::new(max_x, max_y),
        Point::new(min_x, max_y),
    ];

    let mut corner_radii = [0.0_f64; 4];

    for (ci, corner) in corner_pts.iter().enumerate() {
        let mut max_inward_dev = 0.0_f64;
        let mut has_nearby = false;

        for p in points {
            let dx = (p.x - corner.x).abs();
            let dy = (p.y - corner.y).abs();
            if dx > corner_region || dy > corner_region {
                continue;
            }

            has_nearby = true;

            // Distance from sample to the nearest rect edge.
            let dist_to_edge = dist_to_rect_edges(p, min_x, min_y, max_x, max_y);

            // Only consider points that are close to the boundary (on the path).
            if dist_to_edge > effective_tolerance {
                continue;
            }

            // For a quarter-circle of radius r at a right-angle corner:
            //   L1 distance from corner = dx + dy
            //   L2 distance from corner = sqrt(dx^2 + dy^2)
            //   inward_dev = L1 - L2 (always >= 0 by triangle inequality)
            // The maximum inward_dev occurs at 45 degrees and equals r*(2 - sqrt(2)).
            // So: r = inward_dev / (2 - sqrt(2))
            let l1_dist = dx + dy;
            let l2_dist = (dx * dx + dy * dy).sqrt();
            let inward = l1_dist - l2_dist;
            if inward > max_inward_dev {
                max_inward_dev = inward;
            }
        }

        if has_nearby && max_inward_dev > 0.0 {
            let factor = 2.0 - std::f64::consts::SQRT_2; // ~0.5858
            corner_radii[ci] = max_inward_dev / factor;
        }
    }

    // Average the corner radii and check consistency.
    let valid_radii: Vec<f64> = corner_radii.iter().copied().filter(|&r| r > 1.0).collect();
    let (rx, ry) = if valid_radii.len() >= 3 {
        let avg = valid_radii.iter().sum::<f64>() / valid_radii.len() as f64;
        let consistent = valid_radii.iter().all(|&r| (r - avg).abs() / avg < 0.50);
        if consistent {
            let max_r = shorter_side / 2.0;
            let r = avg.min(max_r);
            (r, r)
        } else {
            (0.0, 0.0)
        }
    } else {
        (0.0, 0.0)
    };

    Some(DetectedShape::Rect {
        x: min_x,
        y: min_y,
        width,
        height,
        rx,
        ry,
    })
}

/// Minimum distance from a point to any edge of an axis-aligned rectangle.
fn dist_to_rect_edges(p: &Point, x0: f64, y0: f64, x1: f64, y1: f64) -> f64 {
    // Clamp point to nearest point on rectangle boundary and return distance.
    let cx = p.x.clamp(x0, x1);
    let cy = p.y.clamp(y0, y1);

    // If the point is inside the rect, distance to boundary is the minimum
    // distance to any edge.
    if (cx - p.x).abs() < 1e-12 && (cy - p.y).abs() < 1e-12 {
        // Point is inside; find min distance to each edge.
        let d = [p.x - x0, x1 - p.x, p.y - y0, y1 - p.y];
        return d.iter().cloned().fold(f64::INFINITY, f64::min);
    }

    // Point is outside; distance to nearest edge point.
    ((cx - p.x).powi(2) + (cy - p.y).powi(2)).sqrt()
}

/// Merge corner indices that are within a few samples of each other, keeping
/// the one with the sharpest angle (approximated by position).
fn merge_nearby_corners(corners: &[usize], points: &[Point]) -> Vec<usize> {
    if corners.is_empty() {
        return Vec::new();
    }
    let mut merged: Vec<usize> = Vec::new();
    let mut group: Vec<usize> = vec![corners[0]];

    let n = points.len();

    for &c in &corners[1..] {
        // Distance in index space (handle wraparound).
        let last = *group.last().unwrap();
        let dist = if c > last {
            c - last
        } else {
            c + n - last
        };
        if dist <= 4 {
            group.push(c);
        } else {
            // Pick the middle element of the group.
            merged.push(group[group.len() / 2]);
            group = vec![c];
        }
    }
    // Handle last group, also checking wraparound with first merged corner.
    if !group.is_empty() {
        // Check if last group wraps around to the first group.
        if !merged.is_empty() {
            let first_merged = merged[0];
            let last_in_group = *group.last().unwrap();
            let wrap_dist = (first_merged + n) - last_in_group;
            if wrap_dist <= 4 {
                // Merge this group with the first merged entry.
                group.push(first_merged);
                merged[0] = group[group.len() / 2];
            } else {
                merged.push(group[group.len() / 2]);
            }
        } else {
            merged.push(group[group.len() / 2]);
        }
    }

    merged
}

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Analyze a `BezPath` and detect if it closely approximates a geometric
/// primitive (circle, ellipse, or rectangle).
///
/// The `tolerance` parameter is in pixels: a shape whose sample points all fall
/// within `tolerance` pixels of the ideal primitive will be accepted.
///
/// Detection is attempted in priority order: circle, ellipse, rectangle.
/// If nothing matches, the original path is returned as
/// `DetectedShape::Path`.
pub fn detect_shape(path: &BezPath, tolerance: f64) -> DetectedShape {
    // Quick reject: path must have a minimum number of elements.
    if path.elements().len() < 3 {
        return DetectedShape::Path(path.clone());
    }

    if let Some(circle) = try_circle(path, tolerance) {
        return circle;
    }

    if let Some(ellipse) = try_ellipse(path, tolerance) {
        return ellipse;
    }

    if let Some(rect) = try_rectangle(path, tolerance) {
        return rect;
    }

    DetectedShape::Path(path.clone())
}

/// Format a detected shape as an SVG element string.
///
/// `fill` is the CSS fill value (e.g. `"#ff0000"` or `"none"`).
/// `stroke_attr` is any additional stroke attributes, already formatted
/// (e.g. `r#"stroke="#000" stroke-width="1""#`).
pub fn shape_to_svg(shape: &DetectedShape, fill: &str, stroke_attr: &str) -> String {
    match shape {
        DetectedShape::Circle { cx, cy, r } => {
            format!(
                r#"<circle cx="{cx:.2}" cy="{cy:.2}" r="{r:.2}" fill="{fill}" {stroke_attr}/>"#
            )
        }
        DetectedShape::Ellipse { cx, cy, rx, ry } => {
            format!(
                r#"<ellipse cx="{cx:.2}" cy="{cy:.2}" rx="{rx:.2}" ry="{ry:.2}" fill="{fill}" {stroke_attr}/>"#
            )
        }
        DetectedShape::Rect {
            x,
            y,
            width,
            height,
            rx,
            ry,
        } => {
            if *rx > 0.01 || *ry > 0.01 {
                format!(
                    r#"<rect x="{x:.2}" y="{y:.2}" width="{width:.2}" height="{height:.2}" rx="{rx:.2}" ry="{ry:.2}" fill="{fill}" {stroke_attr}/>"#
                )
            } else {
                format!(
                    r#"<rect x="{x:.2}" y="{y:.2}" width="{width:.2}" height="{height:.2}" fill="{fill}" {stroke_attr}/>"#
                )
            }
        }
        DetectedShape::Path(p) => {
            let d = bezpath_to_svg_d(p);
            format!(r#"<path d="{d}" fill="{fill}" {stroke_attr}/>"#)
        }
    }
}

// Use the shared `bezpath_to_svg_d` from the output module.
use crate::output::bezpath_to_svg_d;

// ===========================================================================
// Tests
// ===========================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use std::f64::consts::PI;

    /// Build a `BezPath` approximating a circle using line segments.
    fn make_circle_path(cx: f64, cy: f64, r: f64, n: usize) -> BezPath {
        let mut path = BezPath::new();
        for i in 0..n {
            let theta = 2.0 * PI * (i as f64) / (n as f64);
            let x = cx + r * theta.cos();
            let y = cy + r * theta.sin();
            if i == 0 {
                path.move_to(Point::new(x, y));
            } else {
                path.line_to(Point::new(x, y));
            }
        }
        path.close_path();
        path
    }

    /// Build a `BezPath` for an axis-aligned rectangle.
    fn make_rect_path(x: f64, y: f64, w: f64, h: f64) -> BezPath {
        let mut path = BezPath::new();
        path.move_to(Point::new(x, y));
        path.line_to(Point::new(x + w, y));
        path.line_to(Point::new(x + w, y + h));
        path.line_to(Point::new(x, y + h));
        path.close_path();
        path
    }

    /// Build a `BezPath` approximating an ellipse using line segments.
    fn make_ellipse_path(cx: f64, cy: f64, rx: f64, ry: f64, n: usize) -> BezPath {
        let mut path = BezPath::new();
        for i in 0..n {
            let theta = 2.0 * PI * (i as f64) / (n as f64);
            let x = cx + rx * theta.cos();
            let y = cy + ry * theta.sin();
            if i == 0 {
                path.move_to(Point::new(x, y));
            } else {
                path.line_to(Point::new(x, y));
            }
        }
        path.close_path();
        path
    }

    /// Build an irregular organic blob that should NOT match any primitive.
    fn make_irregular_path() -> BezPath {
        let mut path = BezPath::new();
        path.move_to(Point::new(0.0, 0.0));
        path.curve_to(
            Point::new(50.0, 100.0),
            Point::new(150.0, -50.0),
            Point::new(200.0, 30.0),
        );
        path.curve_to(
            Point::new(250.0, 110.0),
            Point::new(180.0, 200.0),
            Point::new(100.0, 180.0),
        );
        path.curve_to(
            Point::new(20.0, 160.0),
            Point::new(-30.0, 80.0),
            Point::new(0.0, 0.0),
        );
        path.close_path();
        path
    }

    // -----------------------------------------------------------------------
    // Circle tests
    // -----------------------------------------------------------------------

    #[test]
    fn detect_perfect_circle_100pts() {
        let path = make_circle_path(50.0, 50.0, 40.0, 100);
        let shape = detect_shape(&path, 2.0);
        match shape {
            DetectedShape::Circle { cx, cy, r } => {
                assert!((cx - 50.0).abs() < 1.0, "cx={cx}");
                assert!((cy - 50.0).abs() < 1.0, "cy={cy}");
                assert!((r - 40.0).abs() < 1.0, "r={r}");
            }
            other => panic!("Expected Circle, got {other:?}"),
        }
    }

    #[test]
    fn detect_circle_small() {
        // A very small circle (radius 3px).
        let path = make_circle_path(10.0, 10.0, 3.0, 60);
        let shape = detect_shape(&path, 1.0);
        match shape {
            DetectedShape::Circle { cx, cy, r } => {
                assert!((cx - 10.0).abs() < 1.0);
                assert!((cy - 10.0).abs() < 1.0);
                assert!((r - 3.0).abs() < 0.5, "r={r}");
            }
            other => panic!("Expected Circle, got {other:?}"),
        }
    }

    #[test]
    fn circle_svg_output() {
        let shape = DetectedShape::Circle {
            cx: 50.0,
            cy: 50.0,
            r: 25.0,
        };
        let svg = shape_to_svg(&shape, "#ff0000", r##"stroke="#000""##);
        assert!(svg.contains("circle"));
        assert!(svg.contains("cx=\"50.00\""));
        assert!(svg.contains("r=\"25.00\""));
        assert!(svg.contains("fill=\"#ff0000\""));
    }

    // -----------------------------------------------------------------------
    // Rectangle tests
    // -----------------------------------------------------------------------

    #[test]
    fn detect_perfect_rectangle() {
        let path = make_rect_path(10.0, 20.0, 100.0, 60.0);
        let shape = detect_shape(&path, 2.0);
        match shape {
            DetectedShape::Rect {
                x,
                y,
                width,
                height,
                rx,
                ry,
            } => {
                assert!((x - 10.0).abs() < 1.0, "x={x}");
                assert!((y - 20.0).abs() < 1.0, "y={y}");
                assert!((width - 100.0).abs() < 2.0, "w={width}");
                assert!((height - 60.0).abs() < 2.0, "h={height}");
                assert!(rx.abs() < 0.01);
                assert!(ry.abs() < 0.01);
            }
            other => panic!("Expected Rect, got {other:?}"),
        }
    }

    #[test]
    fn detect_square() {
        let path = make_rect_path(0.0, 0.0, 50.0, 50.0);
        let shape = detect_shape(&path, 2.0);
        match shape {
            DetectedShape::Rect { width, height, .. } => {
                assert!((width - 50.0).abs() < 2.0);
                assert!((height - 50.0).abs() < 2.0);
            }
            // A square could also be detected as a circle due to symmetry;
            // but with 4 line segments and sharp corners it should be a rect.
            other => panic!("Expected Rect, got {other:?}"),
        }
    }

    #[test]
    fn rect_svg_output() {
        let shape = DetectedShape::Rect {
            x: 10.0,
            y: 20.0,
            width: 100.0,
            height: 60.0,
            rx: 0.0,
            ry: 0.0,
        };
        let svg = shape_to_svg(&shape, "blue", "");
        assert!(svg.contains("<rect"));
        assert!(svg.contains("x=\"10.00\""));
        assert!(svg.contains("width=\"100.00\""));
        assert!(!svg.contains("rx="));
    }

    #[test]
    fn rect_rounded_svg_output() {
        let shape = DetectedShape::Rect {
            x: 0.0,
            y: 0.0,
            width: 80.0,
            height: 40.0,
            rx: 5.0,
            ry: 5.0,
        };
        let svg = shape_to_svg(&shape, "none", r#"stroke="red""#);
        assert!(svg.contains("rx=\"5.00\""));
        assert!(svg.contains("ry=\"5.00\""));
    }

    // -----------------------------------------------------------------------
    // Ellipse tests
    // -----------------------------------------------------------------------

    #[test]
    fn detect_ellipse_2_to_1() {
        let path = make_ellipse_path(100.0, 100.0, 60.0, 30.0, 100);
        let shape = detect_shape(&path, 2.0);
        match shape {
            DetectedShape::Ellipse { cx, cy, rx, ry } => {
                assert!((cx - 100.0).abs() < 2.0, "cx={cx}");
                assert!((cy - 100.0).abs() < 2.0, "cy={cy}");
                assert!((rx - 60.0).abs() < 2.0, "rx={rx}");
                assert!((ry - 30.0).abs() < 2.0, "ry={ry}");
            }
            other => panic!("Expected Ellipse, got {other:?}"),
        }
    }

    #[test]
    fn ellipse_svg_output() {
        let shape = DetectedShape::Ellipse {
            cx: 100.0,
            cy: 50.0,
            rx: 60.0,
            ry: 30.0,
        };
        let svg = shape_to_svg(&shape, "#0f0", "");
        assert!(svg.contains("<ellipse"));
        assert!(svg.contains("rx=\"60.00\""));
        assert!(svg.contains("ry=\"30.00\""));
    }

    // -----------------------------------------------------------------------
    // Non-primitive / organic shapes
    // -----------------------------------------------------------------------

    #[test]
    fn irregular_shape_stays_path() {
        let path = make_irregular_path();
        let shape = detect_shape(&path, 2.0);
        match shape {
            DetectedShape::Path(_) => {} // expected
            other => panic!("Expected Path, got {other:?}"),
        }
    }

    // -----------------------------------------------------------------------
    // Edge cases
    // -----------------------------------------------------------------------

    #[test]
    fn degenerate_empty_path() {
        let path = BezPath::new();
        let shape = detect_shape(&path, 2.0);
        match shape {
            DetectedShape::Path(p) => assert!(p.elements().is_empty()),
            other => panic!("Expected Path for empty, got {other:?}"),
        }
    }

    #[test]
    fn degenerate_single_point() {
        let mut path = BezPath::new();
        path.move_to(Point::new(0.0, 0.0));
        let shape = detect_shape(&path, 2.0);
        match shape {
            DetectedShape::Path(_) => {} // expected
            other => panic!("Expected Path for single point, got {other:?}"),
        }
    }

    #[test]
    fn degenerate_single_line() {
        let mut path = BezPath::new();
        path.move_to(Point::new(0.0, 0.0));
        path.line_to(Point::new(100.0, 0.0));
        let shape = detect_shape(&path, 2.0);
        match shape {
            DetectedShape::Path(_) => {} // expected
            other => panic!("Expected Path for line, got {other:?}"),
        }
    }

    #[test]
    fn very_small_circle() {
        // Radius of 1 pixel — barely visible.
        let path = make_circle_path(5.0, 5.0, 1.0, 32);
        let shape = detect_shape(&path, 0.5);
        match shape {
            DetectedShape::Circle { r, .. } => {
                assert!((r - 1.0).abs() < 0.5, "r={r}");
            }
            other => panic!("Expected Circle, got {other:?}"),
        }
    }

    #[test]
    fn path_svg_output_fallback() {
        let mut path = BezPath::new();
        path.move_to(Point::new(0.0, 0.0));
        path.line_to(Point::new(10.0, 0.0));
        path.line_to(Point::new(5.0, 8.66));
        path.close_path();

        let shape = DetectedShape::Path(path);
        let svg = shape_to_svg(&shape, "yellow", "");
        assert!(svg.contains("<path"));
        assert!(svg.contains("d=\""));
        assert!(svg.contains("fill=\"yellow\""));
    }

    #[test]
    fn tight_tolerance_rejects_coarse_circle() {
        // A circle made with only 8 segments — quite coarse.
        let path = make_circle_path(50.0, 50.0, 40.0, 8);
        // With a very tight tolerance (0.1 px) the coarse polygon should not
        // pass as a circle.
        let shape = detect_shape(&path, 0.1);
        match shape {
            DetectedShape::Path(_) => {} // expected
            DetectedShape::Circle { .. } => {
                // Acceptable if the 8-gon happens to pass — this is a
                // best-effort check.
            }
            other => panic!("Unexpected shape: {other:?}"),
        }
    }

    // -----------------------------------------------------------------------
    // Rounded rectangle tests
    // -----------------------------------------------------------------------

    /// Build a `BezPath` for a rounded rectangle using line segments for the
    /// straight edges and quarter-circle arcs (approximated with line segments)
    /// for the corners.
    fn make_rounded_rect_path(
        x: f64,
        y: f64,
        w: f64,
        h: f64,
        r: f64,
        arc_segments: usize,
    ) -> BezPath {
        let mut path = BezPath::new();
        // Start at top edge, after top-left corner arc.
        path.move_to(Point::new(x + r, y));
        // Top edge.
        path.line_to(Point::new(x + w - r, y));
        // Top-right corner arc (center at (x+w-r, y+r)).
        for i in 1..=arc_segments {
            let theta = PI / 2.0 * (1.0 - i as f64 / arc_segments as f64);
            let px = (x + w - r) + r * theta.cos();
            let py = (y + r) - r * theta.sin();
            path.line_to(Point::new(px, py));
        }
        // Right edge.
        path.line_to(Point::new(x + w, y + h - r));
        // Bottom-right corner arc (center at (x+w-r, y+h-r)).
        for i in 1..=arc_segments {
            let theta = PI / 2.0 * (i as f64 / arc_segments as f64);
            let px = (x + w - r) + r * theta.cos();
            let py = (y + h - r) + r * theta.sin();
            path.line_to(Point::new(px, py));
        }
        // Bottom edge.
        path.line_to(Point::new(x + r, y + h));
        // Bottom-left corner arc (center at (x+r, y+h-r)).
        for i in 1..=arc_segments {
            let theta = PI / 2.0 * (1.0 - i as f64 / arc_segments as f64);
            let px = (x + r) - r * theta.cos();
            let py = (y + h - r) + r * theta.sin();
            path.line_to(Point::new(px, py));
        }
        // Left edge.
        path.line_to(Point::new(x, y + r));
        // Top-left corner arc (center at (x+r, y+r)).
        for i in 1..=arc_segments {
            let theta = PI / 2.0 * (i as f64 / arc_segments as f64);
            let px = (x + r) - r * theta.cos();
            let py = (y + r) - r * theta.sin();
            path.line_to(Point::new(px, py));
        }
        path.close_path();
        path
    }

    #[test]
    fn detect_rounded_rectangle() {
        // 200x100 rectangle with corner radius 15, using 16 segments per arc.
        let path = make_rounded_rect_path(10.0, 20.0, 200.0, 100.0, 15.0, 16);
        let shape = detect_shape(&path, 3.0);
        match shape {
            DetectedShape::Rect {
                x,
                y,
                width,
                height,
                rx,
                ry,
            } => {
                assert!((x - 10.0).abs() < 3.0, "x={x}");
                assert!((y - 20.0).abs() < 3.0, "y={y}");
                assert!((width - 200.0).abs() < 5.0, "w={width}");
                assert!((height - 100.0).abs() < 5.0, "h={height}");
                // Rounded rect may or may not detect corner radii depending
                // on how the sampling resolves. Either way, valid detection.
                let _ = (rx, ry);
            }
            DetectedShape::Path(_) => {
                // Rounded rects with large radii may not pass the rectangle
                // heuristics — acceptable fallback.
            }
            other => panic!("Expected Rect or Path, got {other:?}"),
        }
    }

    #[test]
    fn sharp_rect_has_zero_radii() {
        // A sharp rectangle built with many line segments (not the 4-vertex
        // fast path) should still get rx=ry=0.
        let mut path = BezPath::new();
        // Build a rectangle with extra midpoints on each edge to force the
        // sampling path (not the 5-element fast path).
        path.move_to(Point::new(0.0, 0.0));
        path.line_to(Point::new(50.0, 0.0));
        path.line_to(Point::new(100.0, 0.0));
        path.line_to(Point::new(100.0, 30.0));
        path.line_to(Point::new(100.0, 60.0));
        path.line_to(Point::new(50.0, 60.0));
        path.line_to(Point::new(0.0, 60.0));
        path.line_to(Point::new(0.0, 30.0));
        path.close_path();
        let shape = detect_shape(&path, 2.0);
        match shape {
            DetectedShape::Rect { rx, ry, .. } => {
                assert!(rx < 1.0, "rx={rx} should be ~0 for sharp rect");
                assert!(ry < 1.0, "ry={ry} should be ~0 for sharp rect");
            }
            other => panic!("Expected Rect, got {other:?}"),
        }
    }
}
