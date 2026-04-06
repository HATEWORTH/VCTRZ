//! Stage 4: Bezier curve fitting.
//!
//! Converts contour point sequences into smooth Bezier curves.
//!
//! Strategy:
//! 1. Gaussian-weighted smoothing (2 passes) to remove pixel staircase
//! 2. Chaikin corner-cutting subdivision for additional smoothness
//! 3. Curvature-adaptive subsampling to reduce point count
//! 4. Build a polyline BezPath and simplify to cubic Beziers

use kurbo::simplify::{simplify_bezpath, SimplifyOptions};
use kurbo::{BezPath, Point};
use rayon::prelude::*;

use crate::{TracedContour, VectorPath, VectorizeConfig};

/// Compute the turning angle (in radians) at point `i` given neighbours.
/// Returns the angle between vectors (prev→curr) and (curr→next).
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

/// Smooth contour points using a Gaussian-weighted kernel.
/// Applies `passes` iterations. Wraps around for closed contours.
fn gaussian_smooth(points: &[Point], passes: usize) -> Vec<Point> {
    if points.len() < 3 {
        return points.to_vec();
    }

    let mut current = points.to_vec();

    for _ in 0..passes {
        let n = current.len();
        let sigma = f64::max(1.5, n as f64 / 80.0);
        let half_window = (3.0 * sigma).ceil() as usize;

        // Precompute kernel weights
        let mut kernel = Vec::with_capacity(2 * half_window + 1);
        let mut weight_sum = 0.0;
        for offset in 0..=(2 * half_window) {
            let d = offset as f64 - half_window as f64;
            let w = (-0.5 * (d / sigma) * (d / sigma)).exp();
            kernel.push(w);
            weight_sum += w;
        }
        // Normalize
        for w in &mut kernel {
            *w /= weight_sum;
        }

        let mut smoothed = Vec::with_capacity(n);
        for i in 0..n {
            let mut sx = 0.0;
            let mut sy = 0.0;
            for (k, &w) in kernel.iter().enumerate() {
                // Wrap-around index for closed contour
                let idx = (i + n + k).wrapping_sub(half_window) % n;
                sx += current[idx].x * w;
                sy += current[idx].y * w;
            }
            smoothed.push(Point::new(sx, sy));
        }
        current = smoothed;
    }

    current
}

/// Apply Chaikin's corner-cutting subdivision.
/// For each edge (P_i, P_{i+1}), emit Q = 0.75*P_i + 0.25*P_{i+1}
/// and R = 0.25*P_i + 0.75*P_{i+1}.
/// Skips cutting near detected corners (angle > corner_threshold_rad).
fn chaikin_subdivide(
    points: &[Point],
    iterations: usize,
    corner_threshold_rad: f64,
) -> Vec<Point> {
    if points.len() < 3 {
        return points.to_vec();
    }

    let mut current = points.to_vec();

    for _ in 0..iterations {
        let n = current.len();

        // Detect corners: mark points where the turning angle exceeds threshold
        let mut is_corner = vec![false; n];
        for i in 0..n {
            let prev = current[(i + n - 1) % n];
            let curr = current[i];
            let next = current[(i + 1) % n];
            let angle = turning_angle(prev, curr, next);
            if angle > corner_threshold_rad {
                is_corner[i] = true;
            }
        }

        let mut result = Vec::with_capacity(n * 2);
        for i in 0..n {
            let next_i = (i + 1) % n;

            if is_corner[i] || is_corner[next_i] {
                // Keep corners intact — don't cut them
                result.push(current[i]);
            } else {
                let p0 = current[i];
                let p1 = current[next_i];
                // Q = 0.75 * P_i + 0.25 * P_{i+1}
                result.push(Point::new(
                    0.75 * p0.x + 0.25 * p1.x,
                    0.75 * p0.y + 0.25 * p1.y,
                ));
                // R = 0.25 * P_i + 0.75 * P_{i+1}
                result.push(Point::new(
                    0.25 * p0.x + 0.75 * p1.x,
                    0.25 * p0.y + 0.75 * p1.y,
                ));
            }
        }
        current = result;
    }

    current
}

/// Curvature-adaptive subsampling.
/// - Straight sections (angle < 5 deg): keep every 6th point
/// - Moderate curves (5-20 deg): keep every 3rd point
/// - Sharp curves (> 20 deg): keep every point
/// Always keeps the first point.
fn adaptive_subsample(points: &[Point]) -> Vec<Point> {
    if points.len() < 4 {
        return points.to_vec();
    }

    let n = points.len();
    let threshold_low = 5.0_f64.to_radians();
    let threshold_high = 20.0_f64.to_radians();

    let mut result = Vec::with_capacity(n / 2);
    let mut skip_counter = 0_usize;

    for i in 0..n {
        if i == 0 {
            result.push(points[i]);
            skip_counter = 0;
            continue;
        }

        let prev = points[(i + n - 1) % n];
        let curr = points[i];
        let next = points[(i + 1) % n];
        let angle = turning_angle(prev, curr, next);

        let keep_interval = if angle > threshold_high {
            1 // keep every point
        } else if angle > threshold_low {
            3 // keep every 3rd
        } else {
            6 // keep every 6th
        };

        skip_counter += 1;
        if skip_counter >= keep_interval {
            result.push(points[i]);
            skip_counter = 0;
        }
    }

    // Ensure we have at least 3 points for a valid closed path
    if result.len() < 3 && points.len() >= 3 {
        return points.to_vec();
    }

    result
}

/// Build a closed polyline `BezPath` from ordered contour points.
fn polyline_from_points(points: &[Point]) -> BezPath {
    let mut path = BezPath::new();
    if points.is_empty() {
        return path;
    }

    path.move_to(points[0]);
    for &p in &points[1..] {
        path.line_to(p);
    }
    path.close_path();
    path
}

/// Mode-specific fitting parameters.
struct FitParams {
    tolerance: f64,
    corner_threshold_rad: f64,
    smoothing_passes: usize,
    chaikin_iterations: usize,
    use_adaptive_subsample: bool,
}

/// Determine fitting parameters based on mode and quality settings.
fn fit_params_for_mode(config: &VectorizeConfig) -> FitParams {
    use crate::quality::Mode;

    let defaults = VectorizeConfig::default();
    let tolerance = if (config.fit_tolerance - defaults.fit_tolerance).abs() > 1e-9 {
        config.fit_tolerance
    } else {
        config.quality.native_fit_tolerance()
    };
    let corner_threshold_rad = if (config.corner_threshold - defaults.corner_threshold).abs() > 1e-9 {
        config.corner_threshold.to_radians()
    } else {
        config.quality.native_corner_threshold().to_radians()
    };

    match config.mode {
        Mode::Logo => FitParams {
            tolerance,
            corner_threshold_rad: corner_threshold_rad.min(30.0_f64.to_radians()),
            smoothing_passes: 1,    // minimal — preserve hard edges
            chaikin_iterations: 0,  // no corner cutting for geometric shapes
            use_adaptive_subsample: true,
        },
        Mode::Sketch => FitParams {
            tolerance,
            corner_threshold_rad,
            smoothing_passes: 0,    // no smoothing — keep raw line character
            chaikin_iterations: 1,  // light subdivision only
            use_adaptive_subsample: true,
        },
        Mode::Photo => FitParams {
            tolerance,
            corner_threshold_rad,
            smoothing_passes: 3,    // extra smoothing for organic gradients
            chaikin_iterations: 3,  // more subdivision for flowing curves
            use_adaptive_subsample: true,
        },
        Mode::HighFidelity => FitParams {
            tolerance: tolerance * 0.7, // tighter fitting
            corner_threshold_rad,
            smoothing_passes: 2,
            chaikin_iterations: 2,
            use_adaptive_subsample: false, // keep all points for maximum fidelity
        },
        Mode::Illustration => FitParams {
            tolerance,
            corner_threshold_rad,
            smoothing_passes: 2,    // standard
            chaikin_iterations: 2,  // standard
            use_adaptive_subsample: true,
        },
    }
}

/// Fit Bezier curves to all traced contours.
/// Fitting strategy varies by mode:
/// - Logo: minimal smoothing, no Chaikin (preserves hard corners)
/// - Sketch: no smoothing (preserves hand-drawn character)
/// - Photo: extra smoothing + subdivision (flowing organic curves)
/// - HiFi: tight tolerance, keeps all points
/// - Illustration: balanced defaults
pub fn fit_curves(contours: &[TracedContour], config: &VectorizeConfig) -> Vec<VectorPath> {
    let params = fit_params_for_mode(config);

    contours
        .par_iter()
        .map(|contour| {
            let points = &contour.points;

            // Step 1: Gaussian smoothing (mode-controlled passes)
            let smoothed = if params.smoothing_passes > 0 {
                gaussian_smooth(points, params.smoothing_passes)
            } else {
                points.to_vec()
            };

            // Step 2: Chaikin corner-cutting subdivision (mode-controlled iterations)
            let subdivided = if params.chaikin_iterations > 0 {
                chaikin_subdivide(&smoothed, params.chaikin_iterations, params.corner_threshold_rad)
            } else {
                smoothed
            };

            // Step 3: Curvature-adaptive subsampling (skipped for HiFi)
            let subsampled = if params.use_adaptive_subsample {
                adaptive_subsample(&subdivided)
            } else {
                subdivided
            };

            // Step 4: Build polyline and simplify to Bezier curves
            let polyline = polyline_from_points(&subsampled);
            let opts = SimplifyOptions::default();
            let fitted =
                simplify_bezpath(polyline.elements().iter().copied(), params.tolerance, &opts);

            VectorPath {
                path: fitted,
                color: contour.color,
                is_hole: contour.is_hole,
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::f64::consts::PI;

    /// Helper: create a circle of `n` points.
    fn circle_points(n: usize, cx: f64, cy: f64, r: f64) -> Vec<Point> {
        (0..n)
            .map(|i| {
                let angle = 2.0 * PI * i as f64 / n as f64;
                Point::new(cx + r * angle.cos(), cy + r * angle.sin())
            })
            .collect()
    }

    /// Helper: create a square contour (axis-aligned).
    fn square_points(n_per_side: usize) -> Vec<Point> {
        let mut pts = Vec::new();
        // bottom edge
        for i in 0..n_per_side {
            let t = i as f64 / n_per_side as f64;
            pts.push(Point::new(t * 100.0, 0.0));
        }
        // right edge
        for i in 0..n_per_side {
            let t = i as f64 / n_per_side as f64;
            pts.push(Point::new(100.0, t * 100.0));
        }
        // top edge (reversed)
        for i in 0..n_per_side {
            let t = i as f64 / n_per_side as f64;
            pts.push(Point::new(100.0 - t * 100.0, 100.0));
        }
        // left edge (reversed)
        for i in 0..n_per_side {
            let t = i as f64 / n_per_side as f64;
            pts.push(Point::new(0.0, 100.0 - t * 100.0));
        }
        pts
    }

    #[test]
    fn test_gaussian_smooth_preserves_count() {
        let points = circle_points(100, 50.0, 50.0, 40.0);
        let smoothed = gaussian_smooth(&points, 2);
        assert_eq!(smoothed.len(), points.len());
    }

    #[test]
    fn test_gaussian_smooth_reduces_noise() {
        // Circle with pixel-level noise
        let mut points = circle_points(200, 50.0, 50.0, 40.0);
        // Add staircase noise
        for (i, p) in points.iter_mut().enumerate() {
            if i % 2 == 0 {
                p.x += 0.5;
                p.y -= 0.5;
            }
        }

        let smoothed = gaussian_smooth(&points, 2);

        // Measure roughness: sum of angle changes
        let roughness = |pts: &[Point]| -> f64 {
            let n = pts.len();
            (0..n)
                .map(|i| {
                    let prev = pts[(i + n - 1) % n];
                    let curr = pts[i];
                    let next = pts[(i + 1) % n];
                    turning_angle(prev, curr, next)
                })
                .sum::<f64>()
        };

        assert!(
            roughness(&smoothed) < roughness(&points),
            "Gaussian smoothing should reduce roughness"
        );
    }

    #[test]
    fn test_gaussian_smooth_small_input() {
        let points = vec![Point::new(0.0, 0.0), Point::new(1.0, 1.0)];
        let smoothed = gaussian_smooth(&points, 2);
        assert_eq!(smoothed.len(), 2);
        // Should return unchanged since len < 3
        assert_eq!(smoothed[0], points[0]);
    }

    #[test]
    fn test_chaikin_increases_point_count() {
        let points = circle_points(20, 50.0, 50.0, 40.0);
        // Use a high threshold so no corners are detected on a circle
        let subdivided = chaikin_subdivide(&points, 1, PI);
        // Each edge produces 2 points, so n edges -> 2n points
        assert!(
            subdivided.len() >= points.len(),
            "Chaikin should not reduce point count: {} vs {}",
            subdivided.len(),
            points.len()
        );
    }

    #[test]
    fn test_chaikin_preserves_corners() {
        let points = square_points(10);
        // Low threshold to detect the 90-degree corners
        let corner_threshold = 45.0_f64.to_radians();
        let subdivided = chaikin_subdivide(&points, 2, corner_threshold);
        assert!(
            !subdivided.is_empty(),
            "Chaikin should produce non-empty output"
        );
    }

    #[test]
    fn test_chaikin_small_input() {
        let points = vec![Point::new(0.0, 0.0), Point::new(1.0, 1.0)];
        let result = chaikin_subdivide(&points, 2, 1.0);
        assert_eq!(result.len(), 2);
    }

    #[test]
    fn test_adaptive_subsample_reduces_straight() {
        // Long straight line (many colinear points)
        let points: Vec<Point> = (0..120)
            .map(|i| Point::new(i as f64, 0.0))
            .collect();

        let subsampled = adaptive_subsample(&points);
        assert!(
            subsampled.len() < points.len(),
            "Straight sections should be heavily subsampled: {} vs {}",
            subsampled.len(),
            points.len()
        );
    }

    #[test]
    fn test_adaptive_subsample_keeps_curves() {
        // Tight curve: lots of curvature
        let points = circle_points(30, 0.0, 0.0, 5.0);
        let subsampled = adaptive_subsample(&points);
        // A small-radius circle has high curvature, so most points should be kept
        assert!(
            subsampled.len() as f64 > points.len() as f64 * 0.25,
            "High-curvature regions should retain more points: {} vs {}",
            subsampled.len(),
            points.len()
        );
    }

    #[test]
    fn test_adaptive_subsample_small_input() {
        let points = vec![
            Point::new(0.0, 0.0),
            Point::new(1.0, 0.0),
            Point::new(1.0, 1.0),
        ];
        let result = adaptive_subsample(&points);
        assert_eq!(result.len(), 3);
    }

    #[test]
    fn test_polyline_basic() {
        let points = vec![
            Point::new(0.0, 0.0),
            Point::new(10.0, 0.0),
            Point::new(10.0, 10.0),
            Point::new(0.0, 10.0),
        ];
        let path = polyline_from_points(&points);
        // Should have MoveTo + 3 LineTo + ClosePath = 5 elements
        assert_eq!(path.elements().len(), 5);
    }

    #[test]
    fn test_polyline_empty() {
        let path = polyline_from_points(&[]);
        assert!(path.elements().is_empty());
    }

    #[test]
    fn test_simplify_produces_valid_path() {
        let points = circle_points(100, 50.0, 50.0, 40.0);
        let polyline = polyline_from_points(&points);
        let opts = SimplifyOptions::default();
        let simplified =
            simplify_bezpath(polyline.elements().iter().copied(), 2.0, &opts);

        assert!(!simplified.elements().is_empty());
        assert!(matches!(
            simplified.elements()[0],
            kurbo::PathEl::MoveTo(_)
        ));
    }

    #[test]
    fn test_turning_angle_straight() {
        let angle = turning_angle(
            Point::new(0.0, 0.0),
            Point::new(1.0, 0.0),
            Point::new(2.0, 0.0),
        );
        assert!(angle.abs() < 1e-10, "Straight line should have ~0 angle");
    }

    #[test]
    fn test_turning_angle_right_angle() {
        let angle = turning_angle(
            Point::new(0.0, 0.0),
            Point::new(1.0, 0.0),
            Point::new(1.0, 1.0),
        );
        let expected = PI / 2.0;
        assert!(
            (angle - expected).abs() < 1e-10,
            "Expected ~90 degrees, got {:.4} rad",
            angle
        );
    }

    #[test]
    fn test_turning_angle_degenerate() {
        // Zero-length vectors
        let angle = turning_angle(
            Point::new(1.0, 1.0),
            Point::new(1.0, 1.0),
            Point::new(2.0, 2.0),
        );
        assert_eq!(angle, 0.0, "Degenerate input should return 0");
    }

    #[test]
    fn test_full_pipeline_circle() {
        let points = circle_points(200, 50.0, 50.0, 40.0);
        let contour = TracedContour {
            points,
            color: crate::Color::rgb(0, 0, 0),
            is_hole: false,
        };
        let config = VectorizeConfig::default();
        let result = fit_curves(&[contour], &config);
        assert_eq!(result.len(), 1);
        assert!(!result[0].path.elements().is_empty());
        assert_eq!(result[0].is_hole, false);
    }

    #[test]
    fn test_full_pipeline_square() {
        let points = square_points(25);
        let contour = TracedContour {
            points,
            color: crate::Color::rgb(255, 0, 0),
            is_hole: true,
        };
        let config = VectorizeConfig::default();
        let result = fit_curves(&[contour], &config);
        assert_eq!(result.len(), 1);
        assert!(!result[0].path.elements().is_empty());
        assert_eq!(result[0].is_hole, true);
        assert_eq!(result[0].color, crate::Color::rgb(255, 0, 0));
    }

    #[test]
    fn test_full_pipeline_empty() {
        let config = VectorizeConfig::default();
        let result = fit_curves(&[], &config);
        assert!(result.is_empty());
    }

    #[test]
    fn test_full_pipeline_tiny_contour() {
        let points = vec![
            Point::new(0.0, 0.0),
            Point::new(1.0, 0.0),
            Point::new(1.0, 1.0),
            Point::new(0.0, 1.0),
        ];
        let contour = TracedContour {
            points,
            color: crate::Color::rgb(0, 0, 0),
            is_hole: false,
        };
        let config = VectorizeConfig::default();
        let result = fit_curves(&[contour], &config);
        assert_eq!(result.len(), 1);
        assert!(!result[0].path.elements().is_empty());
    }
}
