//! Stage 6: Path and layer optimization.
//!
//! Handles layer ordering based on the configured compositing mode.
//! Stacked mode: largest shapes first (background), smallest on top.
//! Cutout mode: TODO — requires polygon boolean operations.

use kurbo::Shape;

use crate::{LayerMode, VectorPath, VectorizeConfig};

/// Compute the approximate area of a `BezPath` using its bounding box.
/// For more accurate area, we'd use Green's theorem on the path segments,
/// but bbox area is sufficient for z-ordering.
fn approx_area(path: &kurbo::BezPath) -> f64 {
    let bbox = path.bounding_box();
    bbox.width() * bbox.height()
}

/// Optimize paths: layer ordering, degenerate path removal.
pub fn optimize_layers(paths: &[VectorPath], config: &VectorizeConfig) -> Vec<VectorPath> {
    let mut result: Vec<VectorPath> = paths
        .iter()
        // Remove empty or degenerate paths
        .filter(|vp| {
            let elements = vp.path.elements();
            // Must have at least MoveTo + one draw command + ClosePath
            if elements.len() < 3 {
                return false;
            }
            // First element must be MoveTo (not ClosePath)
            if !matches!(elements[0], kurbo::PathEl::MoveTo(_)) {
                return false;
            }
            // Check bounding box is non-trivial
            let bbox = vp.path.bounding_box();
            bbox.width() > 0.5 && bbox.height() > 0.5
        })
        .cloned()
        .collect();

    match config.layer_mode {
        LayerMode::Stacked => {
            // Sort by area descending: largest shapes render first (background),
            // smallest shapes render last (foreground/on top).
            // The background-colored path is naturally the largest and will end up first.
            // Holes should come after their parent shape.
            result.sort_by(|a, b| {
                // Non-holes before holes of similar size
                let a_area = approx_area(&a.path);
                let b_area = approx_area(&b.path);

                match (a.is_hole, b.is_hole) {
                    (false, true) => std::cmp::Ordering::Less,
                    (true, false) => std::cmp::Ordering::Greater,
                    _ => b_area
                        .partial_cmp(&a_area)
                        .unwrap_or(std::cmp::Ordering::Equal),
                }
            });
        }
        LayerMode::Cutout => {
            // TODO: Use polygon boolean operations (clipper2-rust) to
            // compute non-overlapping shape differences.
            // For now, same as stacked ordering.
            result.sort_by(|a, b| {
                let a_area = approx_area(&a.path);
                let b_area = approx_area(&b.path);
                b_area
                    .partial_cmp(&a_area)
                    .unwrap_or(std::cmp::Ordering::Equal)
            });
        }
    }

    result
}
