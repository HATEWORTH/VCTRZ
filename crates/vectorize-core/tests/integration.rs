use image::{DynamicImage, RgbaImage};
use vectorize_core::{VectorizeConfig, vectorize};

/// Create a simple test image: red circle on white background.
fn create_test_image(width: u32, height: u32) -> DynamicImage {
    let mut img = RgbaImage::new(width, height);
    let cx = width as f64 / 2.0;
    let cy = height as f64 / 2.0;
    let radius = (width.min(height) as f64) / 3.0;

    for y in 0..height {
        for x in 0..width {
            let dx = x as f64 - cx;
            let dy = y as f64 - cy;
            let dist = (dx * dx + dy * dy).sqrt();

            if dist < radius {
                img.put_pixel(x, y, image::Rgba([255, 0, 0, 255]));
            } else {
                img.put_pixel(x, y, image::Rgba([255, 255, 255, 255]));
            }
        }
    }

    DynamicImage::ImageRgba8(img)
}

/// Create a multi-color test image: RGB stripes.
fn create_stripe_image(width: u32, height: u32) -> DynamicImage {
    let mut img = RgbaImage::new(width, height);
    let stripe_width = width / 3;

    for y in 0..height {
        for x in 0..width {
            let color = if x < stripe_width {
                image::Rgba([255, 0, 0, 255]) // Red
            } else if x < stripe_width * 2 {
                image::Rgba([0, 255, 0, 255]) // Green
            } else {
                image::Rgba([0, 0, 255, 255]) // Blue
            };
            img.put_pixel(x, y, color);
        }
    }

    DynamicImage::ImageRgba8(img)
}

#[test]
fn test_end_to_end_circle() {
    let img = create_test_image(100, 100);
    let config = VectorizeConfig {
        color_count: 2,
        min_area: 4,
        ..Default::default()
    };

    let svg = vectorize(&img, &config).expect("vectorization should succeed");

    // Basic SVG structure checks
    assert!(svg.contains("<svg"), "SVG should contain <svg tag");
    assert!(svg.contains("</svg>"), "SVG should contain closing tag");
    // VTracer uses width/height attributes
    assert!(
        svg.contains("width=\"100\"") || svg.contains("viewBox=\"0 0 100 100\""),
        "SVG should have dimensions for 100x100 image"
    );

    // Should have at least one path or rect element
    let elem_count = svg.matches("<path").count() + svg.matches("<rect").count();
    assert!(elem_count >= 1, "Expected at least 1 path/rect, got {elem_count}");

    println!("Circle SVG ({} bytes, {} elements):\n{}", svg.len(), elem_count, &svg[..svg.len().min(500)]);
}

#[test]
fn test_end_to_end_stripes() {
    let img = create_stripe_image(90, 60);
    let config = VectorizeConfig {
        color_count: 3,
        min_area: 4,
        ..Default::default()
    };

    let svg = vectorize(&img, &config).expect("vectorization should succeed");

    assert!(svg.contains("<svg"));

    // Should have paths/rects for the 3 color regions.
    let elem_count = svg.matches("<path").count() + svg.matches("<rect").count();
    assert!(elem_count >= 2, "Expected at least 2 paths/rects for 3 stripes, got {elem_count}");

    println!("Stripes SVG ({} bytes, {} elements):\n{}", svg.len(), elem_count, &svg[..svg.len().min(500)]);
}

#[test]
fn test_empty_image_fails() {
    let img = DynamicImage::ImageRgba8(RgbaImage::new(0, 0));
    let config = VectorizeConfig::default();
    assert!(vectorize(&img, &config).is_err());
}

#[test]
fn test_single_color_image() {
    // Solid blue image — should produce minimal SVG
    let mut img = RgbaImage::new(50, 50);
    for pixel in img.pixels_mut() {
        *pixel = image::Rgba([0, 0, 255, 255]);
    }
    let img = DynamicImage::ImageRgba8(img);
    let config = VectorizeConfig {
        color_count: 1,
        min_area: 1,
        ..Default::default()
    };

    let svg = vectorize(&img, &config).expect("should succeed");
    assert!(svg.contains("<svg"));
    println!("Solid SVG: {}", &svg[..svg.len().min(300)]);
}

#[test]
fn test_deterministic_output() {
    // Same image + same config = identical SVG (Sprint 1.1)
    let img = create_test_image(80, 80);
    let config = VectorizeConfig {
        color_count: 4,
        min_area: 4,
        ..Default::default()
    };

    let svg1 = vectorize(&img, &config).expect("run 1");
    let svg2 = vectorize(&img, &config).expect("run 2");
    assert_eq!(svg1, svg2, "Two runs with same input should produce identical SVG");
}
