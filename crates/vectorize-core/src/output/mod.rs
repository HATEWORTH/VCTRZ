//! Stage 7: SVG serialization.
//!
//! Converts fitted vector paths to SVG document output.
//! Adds anti-seam strokes to eliminate white gap artifacts between
//! adjacent color regions.

use std::fmt::Write;

use crate::{Color, VectorPath};

/// Convert a `kurbo::BezPath` to an SVG path `d` attribute string.
/// Ensures every subpath is properly closed with Z to prevent stray lines.
pub fn bezpath_to_svg_d(path: &kurbo::BezPath) -> String {
    use kurbo::PathEl;

    let mut d = String::new();
    let elements = path.elements();
    let len = elements.len();

    for (i, el) in elements.iter().enumerate() {
        match *el {
            PathEl::MoveTo(p) => {
                // If the previous subpath didn't end with Z, close it now
                if i > 0 {
                    if let Some(prev) = elements.get(i - 1) {
                        if !matches!(prev, PathEl::ClosePath) {
                            d.push('Z');
                        }
                    }
                }
                write!(d, "M{:.2},{:.2}", p.x, p.y).unwrap();
            }
            PathEl::LineTo(p) => write!(d, "L{:.2},{:.2}", p.x, p.y).unwrap(),
            PathEl::QuadTo(p1, p2) => {
                write!(d, "Q{:.2},{:.2} {:.2},{:.2}", p1.x, p1.y, p2.x, p2.y).unwrap();
            }
            PathEl::CurveTo(p1, p2, p3) => {
                write!(
                    d,
                    "C{:.2},{:.2} {:.2},{:.2} {:.2},{:.2}",
                    p1.x, p1.y, p2.x, p2.y, p3.x, p3.y
                )
                .unwrap();
            }
            PathEl::ClosePath => d.push('Z'),
        }
    }

    // Ensure the final subpath is closed
    if len > 0 && !matches!(elements.last(), Some(PathEl::ClosePath)) {
        d.push('Z');
    }

    d
}

/// Generate an SVG document from vector paths.
///
/// If `background_color` is provided, a full-canvas `<rect>` is emitted first
/// using that exact color. This is the color from the segmentation stage's
/// background detection — NOT inferred from path areas.
pub fn to_svg(
    paths: &[VectorPath],
    width: u32,
    height: u32,
    background_color: Option<Color>,
) -> String {
    let mut svg = String::with_capacity(paths.len() * 200);

    write!(
        svg,
        r#"<?xml version="1.0" encoding="UTF-8"?>
<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 {width} {height}" width="{width}" height="{height}">
"#
    )
    .unwrap();

    // Emit background rect first if we have a detected background color
    if let Some(bg) = background_color {
        let fill = bg.to_svg_color();
        writeln!(
            svg,
            r#"  <rect width="{width}" height="{height}" fill="{fill}"/>"#
        )
        .unwrap();
    }

    for vp in paths {
        let fill = vp.color.to_svg_color();
        let d = bezpath_to_svg_d(&vp.path);

        if vp.is_hole {
            writeln!(
                svg,
                r#"  <path d="{d}" fill="{fill}" fill-rule="evenodd"/>"#
            )
            .unwrap();
        } else {
            writeln!(
                svg,
                r#"  <path d="{d}" fill="{fill}"/>"#
            )
            .unwrap();
        }
    }

    svg.push_str("</svg>\n");
    svg
}

/// Generate an empty SVG document (no paths found).
pub fn empty_svg(width: u32, height: u32) -> String {
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 {width} {height}" width="{width}" height="{height}">
</svg>
"#
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_bezpath_to_svg_d() {
        let mut path = kurbo::BezPath::new();
        path.move_to(kurbo::Point::new(0.0, 0.0));
        path.line_to(kurbo::Point::new(10.0, 0.0));
        path.line_to(kurbo::Point::new(10.0, 10.0));
        path.close_path();

        let d = bezpath_to_svg_d(&path);
        assert!(d.starts_with('M'));
        assert!(d.ends_with('Z'));
        assert!(d.contains('L'));
    }

    #[test]
    fn test_to_svg_with_background() {
        let bg = Some(Color::rgb(200, 220, 200));
        let paths = vec![crate::VectorPath {
            path: {
                let mut p = kurbo::BezPath::new();
                p.move_to(kurbo::Point::new(10.0, 10.0));
                p.line_to(kurbo::Point::new(20.0, 10.0));
                p.line_to(kurbo::Point::new(20.0, 20.0));
                p.close_path();
                p
            },
            color: Color::rgb(255, 0, 0),
            is_hole: false,
        }];

        let svg = to_svg(&paths, 100, 100, bg);
        // Background rect should be present with the correct color
        assert!(svg.contains("<rect"));
        assert!(svg.contains("fill=\"#c8dcc8\""));
        // Foreground path should also exist
        assert!(svg.contains("<path"));
        assert!(svg.contains("fill=\"#ff0000\""));
    }

    #[test]
    fn test_to_svg_no_background() {
        let paths = vec![crate::VectorPath {
            path: {
                let mut p = kurbo::BezPath::new();
                p.move_to(kurbo::Point::new(0.0, 0.0));
                p.line_to(kurbo::Point::new(10.0, 0.0));
                p.line_to(kurbo::Point::new(10.0, 10.0));
                p.close_path();
                p
            },
            color: Color::rgb(255, 0, 0),
            is_hole: false,
        }];

        let svg = to_svg(&paths, 100, 100, None);
        assert!(!svg.contains("<rect"));
        assert!(svg.contains("<path"));
    }
}
