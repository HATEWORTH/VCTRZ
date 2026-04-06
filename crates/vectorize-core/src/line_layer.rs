//! Binary line-layer extraction for preserving thin high-contrast features.
//!
//! Splits an image into two layers before vectorization:
//! - **Line layer**: Text, outlines, and fine details extracted as a clean binary
//!   image, traced with maximum precision as crisp vector paths.
//! - **Color layer**: The rest of the image with line features inpainted out,
//!   traced normally for smooth fills and gradients.
//!
//! The final SVG composites color paths first, then line paths on top.

use image::{GrayImage, RgbaImage, Luma, Rgba};

/// Result of line extraction: binary mask + detected foreground/background colors.
pub struct LineExtraction {
    /// Binary mask: 255 = line pixel, 0 = background/color pixel.
    pub mask: GrayImage,
    /// Detected foreground (line) color.
    pub fg_color: [u8; 3],
    /// Detected background color.
    pub bg_color: [u8; 3],
    /// Fraction of pixels classified as lines (0.0-1.0).
    pub line_fraction: f64,
}

/// Extract thin high-contrast features (text, outlines) from an RGBA image.
///
/// Returns `None` if no significant line content is detected.
pub fn extract_line_mask(rgba: &RgbaImage) -> Option<LineExtraction> {
    let (w, h) = (rgba.width(), rgba.height());
    if w < 20 || h < 20 {
        return None;
    }

    // Step 1: Convert to grayscale
    let gray = to_grayscale(rgba);

    // Step 2: Adaptive threshold — captures local contrast (text on any background)
    let block_size = adaptive_block_size(w, h);
    let adaptive = adaptive_threshold(&gray, block_size);

    // Step 3: Global Otsu threshold — for reference
    let mut histogram = [0u32; 256];
    for p in gray.pixels() {
        histogram[p[0] as usize] += 1;
    }
    let total: u32 = histogram.iter().sum();
    let otsu = crate::preprocess::find_otsu_threshold(&histogram, total);

    // Step 4: Combine — a pixel is a "line pixel" if:
    //   - It's dark under adaptive threshold (locally high contrast)
    //   - Its absolute brightness is below Otsu (actually dark)
    //   - It's near-achromatic (low saturation) — true text/outlines are black/grey,
    //     not green/orange/colored fills
    let mut mask = GrayImage::new(w, h);
    for y in 0..h {
        for x in 0..w {
            let is_adaptive_dark = adaptive.get_pixel(x, y)[0] == 0;
            let brightness = gray.get_pixel(x, y)[0];
            let is_globally_dark = brightness < otsu.saturating_add(20);

            // Saturation check: colored fills (green, orange, red, blue) have
            // high saturation even when dark. True text/outlines are near-achromatic.
            let p = rgba.get_pixel(x, y);
            let max_ch = p[0].max(p[1]).max(p[2]);
            let min_ch = p[0].min(p[1]).min(p[2]);
            let saturation = max_ch as i32 - min_ch as i32;
            // Allow slightly colored text (e.g., dark brown) but reject clearly chromatic
            let is_achromatic = saturation < 40;

            if is_adaptive_dark && is_globally_dark && is_achromatic {
                mask.put_pixel(x, y, Luma([255]));
            }
        }
    }

    // Step 5: Remove large filled regions — keep only thin features.
    // A thin feature has pixels close to the mask boundary on both sides.
    // Erode then check: if a pixel survives heavy erosion, it's part of a large fill, not a line.
    let eroded = erode_mask(&mask, 6); // 6px erosion — anything surviving is >12px thick
    for y in 0..h {
        for x in 0..w {
            if eroded.get_pixel(x, y)[0] > 0 {
                mask.put_pixel(x, y, Luma([0])); // Remove thick regions
            }
        }
    }

    // Step 6: Clean up noise — remove isolated small clusters
    remove_small_components(&mut mask, 8); // Remove components < 8 pixels

    // Step 7: Dilate by 1px to recapture anti-aliased edges
    let mask = dilate_mask(&mask, 1);

    // Count line pixels
    let line_count = mask.pixels().filter(|p| p[0] > 0).count();
    let total_pixels = (w * h) as usize;
    let line_fraction = line_count as f64 / total_pixels as f64;

    // Reject if too few lines (<0.5%) or too many (>40%)
    if line_fraction < 0.005 || line_fraction > 0.40 {
        tracing::debug!(
            "Line extraction: rejected ({:.1}% line pixels — outside 0.5-40% range)",
            line_fraction * 100.0
        );
        return None;
    }

    // Step 8: Determine foreground and background colors
    let (fg_color, bg_color) = detect_fg_bg_colors(rgba, &mask);

    tracing::info!(
        "Line extraction: {:.1}% line pixels, fg=rgb({},{},{}), bg=rgb({},{},{})",
        line_fraction * 100.0,
        fg_color[0], fg_color[1], fg_color[2],
        bg_color[0], bg_color[1], bg_color[2],
    );

    Some(LineExtraction {
        mask,
        fg_color,
        bg_color,
        line_fraction,
    })
}

/// Build the color-layer image with line features inpainted out.
/// Line pixels are replaced with the average color of nearby non-line pixels.
pub fn build_color_layer(rgba: &RgbaImage, extraction: &LineExtraction) -> RgbaImage {
    let (w, h) = (rgba.width(), rgba.height());
    let mut result = rgba.clone();
    let mask = &extraction.mask;

    // Dilate mask by 2px to also replace anti-aliased fringes around lines
    let expanded_mask = dilate_mask(mask, 2);

    // For each line pixel, replace with average of nearby non-line pixels
    let radius = 5i32;
    for y in 0..h {
        for x in 0..w {
            if expanded_mask.get_pixel(x, y)[0] == 0 {
                continue; // Not a line pixel
            }

            let mut r_sum = 0u32;
            let mut g_sum = 0u32;
            let mut b_sum = 0u32;
            let mut count = 0u32;

            for dy in -radius..=radius {
                for dx in -radius..=radius {
                    let nx = x as i32 + dx;
                    let ny = y as i32 + dy;
                    if nx < 0 || ny < 0 || nx >= w as i32 || ny >= h as i32 {
                        continue;
                    }
                    let nx = nx as u32;
                    let ny = ny as u32;
                    if expanded_mask.get_pixel(nx, ny)[0] == 0 {
                        let p = rgba.get_pixel(nx, ny);
                        r_sum += p[0] as u32;
                        g_sum += p[1] as u32;
                        b_sum += p[2] as u32;
                        count += 1;
                    }
                }
            }

            let pixel = if count > 0 {
                Rgba([
                    (r_sum / count) as u8,
                    (g_sum / count) as u8,
                    (b_sum / count) as u8,
                    255,
                ])
            } else {
                // All neighbors are also lines — use bg color
                Rgba([extraction.bg_color[0], extraction.bg_color[1], extraction.bg_color[2], 255])
            };
            result.put_pixel(x, y, pixel);
        }
    }

    result
}

/// Build a clean binary image for the line layer.
/// Line pixels get the foreground color, non-line pixels get the background color.
pub fn build_line_image(rgba: &RgbaImage, extraction: &LineExtraction) -> RgbaImage {
    let (w, h) = (rgba.width(), rgba.height());
    let mut result = RgbaImage::new(w, h);
    let mask = &extraction.mask;
    let bg = Rgba([extraction.bg_color[0], extraction.bg_color[1], extraction.bg_color[2], 255]);

    for y in 0..h {
        for x in 0..w {
            if mask.get_pixel(x, y)[0] > 0 {
                // Use the actual pixel color (preserves colored text/outlines)
                let p = rgba.get_pixel(x, y);
                result.put_pixel(x, y, *p);
            } else {
                result.put_pixel(x, y, bg);
            }
        }
    }

    result
}

/// Merge two SVG strings: color layer first, then line layer paths on top.
/// Strips the line layer's background elements (rects and the largest fill-color paths).
pub fn merge_svg_layers(
    color_svg: &str,
    line_svg: &str,
    _bg_color: [u8; 3],
) -> String {
    let svg_tag_start = color_svg.find("<svg").unwrap_or(0);
    let header_end = color_svg[svg_tag_start..]
        .find('>')
        .map(|i| svg_tag_start + i + 1)
        .unwrap_or(0);
    let header = &color_svg[..header_end];

    let color_body = extract_svg_body(color_svg);
    let line_body = extract_svg_body(line_svg);

    // Detect the line layer's background color: the first <rect> or <path> fill.
    // VTracer's postprocess adds a background rect; JPEG cleanup may change the color.
    let line_bg_hex = detect_line_bg_color(&line_body);
    let line_bg_rgb = line_bg_hex.as_ref().and_then(|h| parse_hex_rgb(h));

    // Filter out background elements from line layer
    let filtered_line_body: String = line_body
        .lines()
        .filter(|line| {
            let trimmed = line.trim();
            if trimmed.is_empty() {
                return false;
            }
            // Skip all rect elements (background fills)
            if trimmed.starts_with("<rect") {
                return false;
            }
            // Skip paths whose fill is close to the detected line bg color.
            // VTracer can produce slightly different shades across the bg region,
            // so we use fuzzy matching (within 30 per channel).
            if let Some(ref bg_rgb) = line_bg_rgb {
                if let Some(fill_start) = trimmed.find("fill=\"") {
                    let fill_val = &trimmed[fill_start + 6..];
                    if let Some(end) = fill_val.find('"') {
                        let fill = &fill_val[..end];
                        if let Some(fill_rgb) = parse_hex_rgb(fill) {
                            let dr = (fill_rgb[0] as i32 - bg_rgb[0] as i32).abs();
                            let dg = (fill_rgb[1] as i32 - bg_rgb[1] as i32).abs();
                            let db = (fill_rgb[2] as i32 - bg_rgb[2] as i32).abs();
                            if dr <= 30 && dg <= 30 && db <= 30 {
                                return false; // Close to bg color — skip
                            }
                        }
                    }
                }
            }
            true
        })
        .collect::<Vec<_>>()
        .join("\n");

    let mut svg = String::with_capacity(color_svg.len() + filtered_line_body.len());
    svg.push_str(header);
    svg.push('\n');
    svg.push_str(&color_body);
    if !filtered_line_body.trim().is_empty() {
        svg.push_str("\n<g id=\"lines\">\n");
        svg.push_str(&filtered_line_body);
        svg.push_str("\n</g>\n");
    }
    svg.push_str("</svg>");

    svg
}

/// Parse a hex color string like `#RRGGBB` into [R, G, B].
fn parse_hex_rgb(s: &str) -> Option<[u8; 3]> {
    let s = s.strip_prefix('#')?;
    if s.len() != 6 {
        return None;
    }
    let r = u8::from_str_radix(&s[0..2], 16).ok()?;
    let g = u8::from_str_radix(&s[2..4], 16).ok()?;
    let b = u8::from_str_radix(&s[4..6], 16).ok()?;
    Some([r, g, b])
}

/// Detect the background color of a line SVG by reading the first rect or path fill.
fn detect_line_bg_color(svg_body: &str) -> Option<String> {
    // Look for the first <rect fill="..."> or first <path fill="...">
    for line in svg_body.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with("<rect") || trimmed.starts_with("<path") {
            if let Some(fill_start) = trimmed.find("fill=\"") {
                let fill_val = &trimmed[fill_start + 6..];
                if let Some(end) = fill_val.find('"') {
                    return Some(fill_val[..end].to_string());
                }
            }
        }
    }
    None
}

// ── Internal helpers ─────────────────────────────────────────────

/// Extract the body of an SVG (everything between the <svg ...> opening tag and </svg>).
/// Skips XML declaration and comments before <svg>, and the <svg> tag itself.
fn extract_svg_body(svg: &str) -> String {
    // Find the <svg ...> opening tag and skip past its closing >
    let svg_tag_start = svg.find("<svg").unwrap_or(0);
    let start = svg[svg_tag_start..].find('>').map(|i| svg_tag_start + i + 1).unwrap_or(0);
    let end = svg.rfind("</svg>").unwrap_or(svg.len());
    svg[start..end].to_string()
}

/// Convert RGBA to grayscale using luminance weights.
fn to_grayscale(rgba: &RgbaImage) -> GrayImage {
    let (w, h) = (rgba.width(), rgba.height());
    let mut gray = GrayImage::new(w, h);
    for y in 0..h {
        for x in 0..w {
            let p = rgba.get_pixel(x, y);
            let lum = (p[0] as u32 * 299 + p[1] as u32 * 587 + p[2] as u32 * 114) / 1000;
            gray.put_pixel(x, y, Luma([lum as u8]));
        }
    }
    gray
}

/// Adaptive threshold using local mean.
/// Each pixel is compared to the mean of its `block_size x block_size` neighborhood.
/// If the pixel is darker than (mean - offset), it's set to 0 (dark/foreground).
fn adaptive_threshold(gray: &GrayImage, block_size: u32) -> GrayImage {
    let (w, h) = (gray.width(), gray.height());
    let half = (block_size / 2) as i32;
    let offset = 12i32; // Bias: pixel must be this much darker than local mean

    // Compute integral image for fast mean computation
    let mut integral = vec![0i64; (w as usize + 1) * (h as usize + 1)];
    let iw = w as usize + 1;
    for y in 0..h as usize {
        let mut row_sum = 0i64;
        for x in 0..w as usize {
            row_sum += gray.get_pixel(x as u32, y as u32)[0] as i64;
            integral[(y + 1) * iw + (x + 1)] = row_sum + integral[y * iw + (x + 1)];
        }
    }

    let mut result = GrayImage::new(w, h);
    for y in 0..h as i32 {
        for x in 0..w as i32 {
            let x1 = (x - half).max(0) as usize;
            let y1 = (y - half).max(0) as usize;
            let x2 = (x + half).min(w as i32 - 1) as usize + 1;
            let y2 = (y + half).min(h as i32 - 1) as usize + 1;

            let area = ((x2 - x1) * (y2 - y1)) as i64;
            let sum = integral[y2 * iw + x2] - integral[y1 * iw + x2]
                - integral[y2 * iw + x1] + integral[y1 * iw + x1];
            let mean = sum / area.max(1);

            let pixel_val = gray.get_pixel(x as u32, y as u32)[0] as i32;
            if pixel_val < (mean as i32 - offset) {
                result.put_pixel(x as u32, y as u32, Luma([0])); // Dark = foreground
            } else {
                result.put_pixel(x as u32, y as u32, Luma([255])); // Light = background
            }
        }
    }

    result
}

/// Choose adaptive threshold block size based on image dimensions.
fn adaptive_block_size(w: u32, h: u32) -> u32 {
    let min_dim = w.min(h);
    // Block should be ~3-5% of image dimension, odd number, min 15
    let size = (min_dim / 25).max(15);
    size | 1 // Ensure odd
}

/// Erode a binary mask by `radius` pixels (shrink white regions).
fn erode_mask(mask: &GrayImage, radius: u32) -> GrayImage {
    let (w, h) = (mask.width(), mask.height());
    let mut result = mask.clone();
    let r = radius as i32;

    for y in 0..h {
        for x in 0..w {
            if mask.get_pixel(x, y)[0] == 0 {
                continue;
            }
            // Check if any pixel in the radius is background
            let mut is_boundary = false;
            'outer: for dy in -r..=r {
                for dx in -r..=r {
                    if dx * dx + dy * dy > r * r {
                        continue;
                    }
                    let nx = x as i32 + dx;
                    let ny = y as i32 + dy;
                    if nx < 0 || ny < 0 || nx >= w as i32 || ny >= h as i32 {
                        is_boundary = true;
                        break 'outer;
                    }
                    if mask.get_pixel(nx as u32, ny as u32)[0] == 0 {
                        is_boundary = true;
                        break 'outer;
                    }
                }
            }
            if is_boundary {
                result.put_pixel(x, y, Luma([0]));
            }
        }
    }
    result
}

/// Dilate a binary mask by `radius` pixels (expand white regions).
fn dilate_mask(mask: &GrayImage, radius: u32) -> GrayImage {
    let (w, h) = (mask.width(), mask.height());
    let mut result = mask.clone();
    let r = radius as i32;

    for y in 0..h {
        for x in 0..w {
            if mask.get_pixel(x, y)[0] > 0 {
                continue; // Already set
            }
            // Check if any pixel in the radius is foreground
            let mut has_fg = false;
            'outer: for dy in -r..=r {
                for dx in -r..=r {
                    if dx * dx + dy * dy > r * r {
                        continue;
                    }
                    let nx = x as i32 + dx;
                    let ny = y as i32 + dy;
                    if nx < 0 || ny < 0 || nx >= w as i32 || ny >= h as i32 {
                        continue;
                    }
                    if mask.get_pixel(nx as u32, ny as u32)[0] > 0 {
                        has_fg = true;
                        break 'outer;
                    }
                }
            }
            if has_fg {
                result.put_pixel(x, y, Luma([255]));
            }
        }
    }
    result
}

/// Remove connected components smaller than `min_size` pixels.
fn remove_small_components(mask: &mut GrayImage, min_size: usize) {
    let (w, h) = (mask.width() as usize, mask.height() as usize);
    let mut labels = vec![0u32; w * h];
    let mut next_label = 1u32;
    let mut component_sizes: Vec<usize> = vec![0]; // label 0 = background

    for y in 0..h {
        for x in 0..w {
            if mask.get_pixel(x as u32, y as u32)[0] == 0 || labels[y * w + x] > 0 {
                continue;
            }

            // BFS flood fill
            let label = next_label;
            next_label += 1;
            let mut size = 0usize;
            let mut queue = std::collections::VecDeque::new();
            queue.push_back((x, y));
            labels[y * w + x] = label;

            while let Some((cx, cy)) = queue.pop_front() {
                size += 1;
                for (nx, ny) in [
                    (cx.wrapping_sub(1), cy),
                    (cx + 1, cy),
                    (cx, cy.wrapping_sub(1)),
                    (cx, cy + 1),
                ] {
                    if nx < w && ny < h && labels[ny * w + nx] == 0
                        && mask.get_pixel(nx as u32, ny as u32)[0] > 0
                    {
                        labels[ny * w + nx] = label;
                        queue.push_back((nx, ny));
                    }
                }
            }

            component_sizes.push(size);
        }
    }

    // Zero out small components
    let mut removed = 0usize;
    for y in 0..h {
        for x in 0..w {
            let label = labels[y * w + x] as usize;
            if label > 0 && component_sizes[label] < min_size {
                mask.put_pixel(x as u32, y as u32, Luma([0]));
                removed += 1;
            }
        }
    }

    if removed > 0 {
        tracing::debug!("Line extraction: removed {} noise pixels in small components", removed);
    }
}

/// Detect the foreground (line) and background colors from the image + mask.
fn detect_fg_bg_colors(rgba: &RgbaImage, mask: &GrayImage) -> ([u8; 3], [u8; 3]) {
    let mut fg_r = 0u64;
    let mut fg_g = 0u64;
    let mut fg_b = 0u64;
    let mut fg_count = 0u64;

    let mut bg_r = 0u64;
    let mut bg_g = 0u64;
    let mut bg_b = 0u64;
    let mut bg_count = 0u64;

    // Sample every 3rd pixel for speed
    for (x, y, p) in rgba.enumerate_pixels() {
        if (x + y) % 3 != 0 {
            continue;
        }
        if mask.get_pixel(x, y)[0] > 0 {
            fg_r += p[0] as u64;
            fg_g += p[1] as u64;
            fg_b += p[2] as u64;
            fg_count += 1;
        } else {
            bg_r += p[0] as u64;
            bg_g += p[1] as u64;
            bg_b += p[2] as u64;
            bg_count += 1;
        }
    }

    let fg = if fg_count > 0 {
        [
            (fg_r / fg_count) as u8,
            (fg_g / fg_count) as u8,
            (fg_b / fg_count) as u8,
        ]
    } else {
        [0, 0, 0]
    };

    let bg = if bg_count > 0 {
        [
            (bg_r / bg_count) as u8,
            (bg_g / bg_count) as u8,
            (bg_b / bg_count) as u8,
        ]
    } else {
        [255, 255, 255]
    };

    (fg, bg)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_to_grayscale() {
        let mut img = RgbaImage::new(2, 2);
        img.put_pixel(0, 0, Rgba([255, 255, 255, 255]));
        img.put_pixel(1, 0, Rgba([0, 0, 0, 255]));
        let gray = to_grayscale(&img);
        assert_eq!(gray.get_pixel(0, 0)[0], 255);
        assert_eq!(gray.get_pixel(1, 0)[0], 0);
    }

    #[test]
    fn test_adaptive_block_size() {
        assert!(adaptive_block_size(100, 100) >= 15);
        assert!(adaptive_block_size(100, 100) % 2 == 1); // Must be odd
        assert!(adaptive_block_size(1000, 1000) > adaptive_block_size(100, 100));
    }

    #[test]
    fn test_extract_svg_body() {
        let svg = "<svg width=\"100\" height=\"100\"><path d=\"M0 0\"/></svg>";
        let body = extract_svg_body(svg);
        assert!(body.contains("<path"));
        assert!(!body.contains("<svg"));
        assert!(!body.contains("</svg>"));
    }

    #[test]
    fn test_dilate_erode_inverse() {
        let mut mask = GrayImage::new(20, 20);
        // Draw a small dot
        mask.put_pixel(10, 10, Luma([255]));
        mask.put_pixel(11, 10, Luma([255]));
        mask.put_pixel(10, 11, Luma([255]));
        mask.put_pixel(11, 11, Luma([255]));

        let dilated = dilate_mask(&mask, 2);
        // Dilated should be larger
        let orig_count: usize = mask.pixels().filter(|p| p[0] > 0).count();
        let dil_count: usize = dilated.pixels().filter(|p| p[0] > 0).count();
        assert!(dil_count > orig_count);
    }
}
