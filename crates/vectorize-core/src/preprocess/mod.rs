//! Stage 1: Image preprocessing.
//!
//! - Alpha handling (transparency detection, premultiply against white)
//! - JPEG artifact cleanup for high-contrast images (brightness threshold)
//! - Edge-based background separation (flood-fill from borders)

use image::{DynamicImage, RgbaImage};

use crate::{PreparedImage, VectorizeConfig};

/// Prepare the image for vectorization.
///
/// Mode-specific preprocessing:
/// - **Logo**: JPEG artifact cleanup always runs, skip tonal bands (flat colors)
/// - **Sketch**: High-contrast boost, skip tonal bands
/// - **Photo**: Full tonal band processing, skip JPEG cleanup (preserve gradients)
/// - **HiFi**: Minimal preprocessing to preserve every detail
/// - **Illustration**: Standard preprocessing pipeline
pub fn prepare(image: &DynamicImage, config: &VectorizeConfig) -> PreparedImage {
    use crate::quality::Mode;

    let mut rgba = image.to_rgba8();
    let width = rgba.width();
    let height = rgba.height();
    let pixel_count = (width as usize) * (height as usize);

    let opaque_mask = if config.skip_transparent {
        let mask = build_mask_and_flatten(&mut rgba, pixel_count);
        if let Some(mut mask) = mask {
            remove_opaque_background(&rgba, &mut mask, width, height);
            Some(mask)
        } else {
            None
        }
    } else {
        flatten_semitransparent(&mut rgba);
        None
    };

    // Logo mode: hard threshold to eliminate anti-aliasing.
    // Snaps every pixel to its nearest quantized color — no gray fringes.
    // Then applies edge smoothing (controlled by edge_smoothing slider).
    if config.mode == Mode::Logo {
        logo_threshold(&mut rgba, config);
    }

    // JPEG artifact cleanup — mode-controlled.
    // Logo: skip (already thresholded above).
    // Photo/HiFi: skip (preserve gradient subtlety).
    let was_jpeg_cleaned = match config.mode {
        Mode::Logo | Mode::Photo | Mode::HighFidelity => false,
        _ => clean_jpeg_artifacts(&mut rgba),
    };

    // Separate subject from background using color-based flood fill from corners.
    // Skip if JPEG cleanup already ran (the image is already clean).
    // Skip if there's already a mask (from alpha handling).
    let opaque_mask = if !was_jpeg_cleaned && opaque_mask.is_none() {
        separate_background(&rgba, opaque_mask, width, height)
    } else {
        opaque_mask
    };

    // Smooth the subject/background boundary to eliminate jagged pixel edges.
    // HiFi: skip boundary smoothing to preserve pixel-level detail.
    if let Some(ref mask) = opaque_mask {
        if config.mode != Mode::HighFidelity {
            smooth_mask_boundary(&mut rgba, mask, width, height);
        }
    }

    // Tonal band preprocessing — mode-controlled.
    // Logo/Sketch: skip (flat colors, no gradient bands to manage).
    // Photo/Illustration/HiFi: apply if user has adjusted tonal sliders.
    let apply_tonal = match config.mode {
        Mode::Logo | Mode::Sketch => false,
        _ => config.quality.needs_tonal_preprocessing(),
    };
    if apply_tonal {
        apply_tonal_bands(&mut rgba, &config.quality);
    }

    // Achromatic contrast boost: stretch luminance range in low-saturation
    // regions so VTracer allocates more gradient layers for grays/whites.
    // Without this, white-to-gray gradients band visibly while saturated
    // colors blend smoothly (because VTracer's layer_difference is uniform).
    if config.mode != Mode::Logo && config.mode != Mode::Sketch {
        boost_achromatic_contrast(&mut rgba);
    }

    // Edge smoothing for non-Logo modes: light Gaussian blur to smooth
    // jagged pixel boundaries before VTracer traces them.
    // Logo mode handles this differently (blur→threshold in logo_threshold).
    if config.mode != Mode::Logo && config.edge_smoothing > 0.01 {
        pre_blur(&mut rgba, config.edge_smoothing);
    }

    PreparedImage {
        image: rgba,
        opaque_mask,
        width,
        height,
    }
}

// ── Mask boundary smoothing ───────────────────────────────────────

/// Smooth the RGBA pixels along the mask boundary so the subject has
/// anti-aliased edges. Finds pixels within 2px of the mask edge and
/// blends them with their opaque neighbors, creating a smooth transition
/// that VTracer traces as clean curves instead of pixel staircases.
fn smooth_mask_boundary(rgba: &mut RgbaImage, mask: &[bool], width: u32, height: u32) {
    let w = width as usize;
    let h = height as usize;

    // Find boundary pixels: opaque pixels adjacent to masked-out pixels
    let mut is_boundary = vec![false; w * h];
    let mut boundary_count = 0u32;

    for y in 1..h - 1 {
        for x in 1..w - 1 {
            let idx = y * w + x;
            if !mask[idx] {
                continue; // Skip masked-out pixels
            }
            // Check if any 8-neighbor is masked out
            let has_bg_neighbor = [
                (x - 1, y - 1), (x, y - 1), (x + 1, y - 1),
                (x - 1, y),                  (x + 1, y),
                (x - 1, y + 1), (x, y + 1), (x + 1, y + 1),
            ]
            .iter()
            .any(|&(nx, ny)| !mask[ny * w + nx]);

            if has_bg_neighbor {
                is_boundary[idx] = true;
                boundary_count += 1;
            }
        }
    }

    if boundary_count == 0 {
        return;
    }

    // For each boundary pixel, average its color with neighboring opaque pixels.
    // This creates a 1px anti-aliased edge.
    let original: Vec<[u8; 4]> = rgba.pixels().map(|p| [p[0], p[1], p[2], p[3]]).collect();

    for y in 1..h - 1 {
        for x in 1..w - 1 {
            let idx = y * w + x;
            if !is_boundary[idx] {
                continue;
            }

            // Weighted average of this pixel and its opaque neighbors
            let mut r_sum = original[idx][0] as u32 * 4;
            let mut g_sum = original[idx][1] as u32 * 4;
            let mut b_sum = original[idx][2] as u32 * 4;
            let mut weight = 4u32;

            for &(nx, ny) in &[
                (x - 1, y - 1), (x, y - 1), (x + 1, y - 1),
                (x - 1, y),                  (x + 1, y),
                (x - 1, y + 1), (x, y + 1), (x + 1, y + 1),
            ] {
                let nidx = ny * w + nx;
                if mask[nidx] {
                    // Opaque neighbor — include in average
                    let w = if nx == x || ny == y { 2 } else { 1 }; // Cardinal > diagonal
                    r_sum += original[nidx][0] as u32 * w;
                    g_sum += original[nidx][1] as u32 * w;
                    b_sum += original[nidx][2] as u32 * w;
                    weight += w;
                }
            }

            let pixel = rgba.get_pixel_mut(x as u32, y as u32);
            pixel[0] = (r_sum / weight) as u8;
            pixel[1] = (g_sum / weight) as u8;
            pixel[2] = (b_sum / weight) as u8;
        }
    }

    tracing::debug!("Boundary smoothing: softened {} edge pixels", boundary_count);
}

// ── Tonal band preprocessing ──────────────────────────────────────

/// Compress luminance range within shadow, midtone, and highlight bands.
/// Operates in Oklab space: squeezes L (lightness) toward the band's center
/// point, reducing tonal variation without creating hard banding edges.
///
/// At detail=100: no change (full pass-through).
/// At detail=0: all pixels in the band collapse to the band's center luminance.
/// In between: smooth compression — nearby tones merge while overall shape is preserved.
///
/// This reduces the number of distinct colors vtracer sees, which means fewer
/// regions to trace, without creating the banding artifacts that discrete
/// level-quantization produces.
fn apply_tonal_bands(rgba: &mut RgbaImage, quality: &crate::quality::QualitySettings) {
    use palette::{FromColor, Oklab, Srgb};

    let (shadow_strength, mid_strength, hi_strength) = quality.tonal_compression();

    // Shadow: L in [0.0, 0.35), Midtone: [0.35, 0.70), Highlight: [0.70, 1.0]
    const SHADOW_MAX: f32 = 0.35;
    const HIGHLIGHT_MIN: f32 = 0.70;

    // Center points — where each band compresses toward.
    // Shadows compress toward near-black, highlights toward near-white,
    // midtones toward the band center.
    const SHADOW_CENTER: f32 = 0.12;
    const MID_CENTER: f32 = 0.525;
    const HIGHLIGHT_CENTER: f32 = 0.88;

    // Soft crossfade width at band boundaries to avoid hard edges.
    const FADE_WIDTH: f32 = 0.05;

    let mut affected = 0u32;

    for pixel in rgba.pixels_mut() {
        if pixel[3] == 0 {
            continue;
        }

        let srgb = Srgb::new(
            pixel[0] as f32 / 255.0,
            pixel[1] as f32 / 255.0,
            pixel[2] as f32 / 255.0,
        );
        let oklab: Oklab = Oklab::from_color(srgb);
        let l = oklab.l;

        // Compute compression for each band, with soft crossfade at boundaries.
        // Each band contributes a "pull" toward its center, weighted by how
        // much the pixel belongs to that band (1.0 = fully inside, 0.0 = outside).
        let shadow_weight = if l < SHADOW_MAX - FADE_WIDTH {
            1.0
        } else if l < SHADOW_MAX + FADE_WIDTH {
            1.0 - (l - (SHADOW_MAX - FADE_WIDTH)) / (2.0 * FADE_WIDTH)
        } else {
            0.0
        };

        let highlight_weight = if l > HIGHLIGHT_MIN + FADE_WIDTH {
            1.0
        } else if l > HIGHLIGHT_MIN - FADE_WIDTH {
            (l - (HIGHLIGHT_MIN - FADE_WIDTH)) / (2.0 * FADE_WIDTH)
        } else {
            0.0
        };

        let mid_weight = (1.0 - shadow_weight - highlight_weight).max(0.0);

        // Compute the blended compression strength and target center.
        let strength = shadow_weight * shadow_strength
            + mid_weight * mid_strength
            + highlight_weight * hi_strength;

        if strength < 0.001 {
            continue;
        }

        let center = shadow_weight * SHADOW_CENTER
            + mid_weight * MID_CENTER
            + highlight_weight * HIGHLIGHT_CENTER;

        // Compress: lerp L toward the center by `strength`.
        // strength=0 → no change, strength=1 → fully collapsed to center.
        let new_l = l + (center - l) * strength;

        if (new_l - l).abs() < 0.001 {
            continue;
        }

        // Also compress chrominance (a, b) proportionally — this merges
        // similar hue variations within the band, reducing distinct colors.
        let chroma_compress = strength * 0.5; // Softer on chrominance than luminance
        let new_a = oklab.a * (1.0 - chroma_compress);
        let new_b = oklab.b * (1.0 - chroma_compress);

        let new_oklab = Oklab::new(new_l.clamp(0.0, 1.0), new_a, new_b);
        let new_srgb: Srgb = Srgb::from_color(new_oklab);
        pixel[0] = (new_srgb.red.clamp(0.0, 1.0) * 255.0).round() as u8;
        pixel[1] = (new_srgb.green.clamp(0.0, 1.0) * 255.0).round() as u8;
        pixel[2] = (new_srgb.blue.clamp(0.0, 1.0) * 255.0).round() as u8;
        affected += 1;
    }

    if affected > 0 {
        tracing::info!(
            "Tonal compression: adjusted {} pixels (shadow={:.0}% mid={:.0}% hi={:.0}%)",
            affected,
            shadow_strength * 100.0,
            mid_strength * 100.0,
            hi_strength * 100.0,
        );
    }
}

// ── Alpha handling ──────────────────────────────────────────────────

#[inline]
fn premultiply_white(channel: u8, alpha: u8) -> u8 {
    let c = channel as u16;
    let a = alpha as u16;
    ((c * a + 255 * (255 - a)) / 255) as u8
}

fn build_mask_and_flatten(rgba: &mut RgbaImage, pixel_count: usize) -> Option<Vec<bool>> {
    let mut mask = Vec::with_capacity(pixel_count);
    let mut has_transparent = false;

    for pixel in rgba.pixels_mut() {
        let a = pixel[3];
        if a == 0 {
            has_transparent = true;
            mask.push(false);
        } else {
            if a < 255 {
                pixel[0] = premultiply_white(pixel[0], a);
                pixel[1] = premultiply_white(pixel[1], a);
                pixel[2] = premultiply_white(pixel[2], a);
                pixel[3] = 255;
            }
            mask.push(true);
        }
    }

    if has_transparent { Some(mask) } else { None }
}

fn flatten_semitransparent(rgba: &mut RgbaImage) {
    for pixel in rgba.pixels_mut() {
        let a = pixel[3];
        if a > 0 && a < 255 {
            pixel[0] = premultiply_white(pixel[0], a);
            pixel[1] = premultiply_white(pixel[1], a);
            pixel[2] = premultiply_white(pixel[2], a);
            pixel[3] = 255;
        }
    }
}

fn remove_opaque_background(
    rgba: &RgbaImage,
    mask: &mut [bool],
    width: u32,
    height: u32,
) {
    let w = width as usize;
    let h = height as usize;

    // Skip on tiny images — flood fill would erase everything.
    if w < 4 || h < 4 {
        return;
    }

    let mut border_colors: Vec<[u8; 3]> = Vec::new();
    for y in 0..h {
        for x in 0..w {
            let idx = y * w + x;
            if !mask[idx] { continue; }
            let has_transparent_neighbor = neighbors_4(x, y, w, h)
                .iter()
                .any(|&(nx, ny)| !mask[ny * w + nx]);
            if has_transparent_neighbor {
                let pixel = rgba.get_pixel(x as u32, y as u32);
                border_colors.push([pixel[0], pixel[1], pixel[2]]);
            }
        }
    }

    if border_colors.is_empty() { return; }

    let bg_color = most_common_color(&border_colors);
    let tolerance = 30u16;
    let mut changed = true;
    let mut iterations = 0;
    while changed && iterations < 20 {
        changed = false;
        iterations += 1;
        for y in 0..h {
            for x in 0..w {
                let idx = y * w + x;
                if !mask[idx] { continue; }
                let has_transparent_neighbor = neighbors_4(x, y, w, h)
                    .iter()
                    .any(|&(nx, ny)| !mask[ny * w + nx]);
                if !has_transparent_neighbor { continue; }
                let pixel = rgba.get_pixel(x as u32, y as u32);
                if color_distance_rgb([pixel[0], pixel[1], pixel[2]], bg_color) <= tolerance {
                    mask[idx] = false;
                    changed = true;
                }
            }
        }
    }
}

// ── Logo-mode hard thresholding ─────────────────────────────────────

/// Hard black/white threshold for Logo mode.
///
/// 1. Compute brightness of each pixel
/// 2. Apply blur to smooth jagged pixel boundaries
/// 3. Threshold: everything above cutoff → white, below → black
///
/// `color_threshold` (0-100) controls where the cutoff sits:
///   0 = low cutoff (more black), 50 = Otsu auto, 100 = high cutoff (more white)
///
/// `edge_smoothing` controls blur radius before thresholding:
///   0 = raw pixels (jagged), 1-3 = smooth edges
fn logo_threshold(rgba: &mut RgbaImage, config: &crate::VectorizeConfig) {
    let width = rgba.width() as usize;
    let height = rgba.height() as usize;
    if width * height < 4 { return; }

    // Build brightness histogram for Otsu baseline
    let mut histogram = [0u32; 256];
    for pixel in rgba.pixels() {
        let brightness = ((pixel[0] as u32 * 299 + pixel[1] as u32 * 587 + pixel[2] as u32 * 114) / 1000) as u8;
        histogram[brightness as usize] += 1;
    }
    let total: u32 = histogram.iter().sum();

    // Otsu's method finds the natural black/white split point
    let otsu = find_otsu_threshold(&histogram, total);

    // User's color_threshold shifts the cutoff:
    // 0 = cutoff at 25% brightness (more black)
    // 50 = Otsu auto (natural split)
    // 100 = cutoff at 75% brightness (more white)
    let t = config.color_threshold.clamp(0.0, 100.0) as f64;
    let cutoff = if (t - 50.0).abs() < 1.0 {
        otsu
    } else if t < 50.0 {
        // Shift toward darker cutoff (more becomes black)
        let shift = (50.0 - t) / 50.0; // 0-1
        let target = otsu as f64 * (1.0 - shift * 0.6); // up to 60% lower
        target.round().clamp(10.0, 245.0) as u8
    } else {
        // Shift toward brighter cutoff (more becomes white)
        let shift = (t - 50.0) / 50.0; // 0-1
        let target = otsu as f64 + (255.0 - otsu as f64) * shift * 0.6;
        target.round().clamp(10.0, 245.0) as u8
    };

    tracing::info!("Logo threshold: cutoff={} (otsu={}, slider={:.0})", cutoff, otsu, t);

    // Step 1: Convert to grayscale brightness, then snap to pure B&W
    for pixel in rgba.pixels_mut() {
        let brightness = ((pixel[0] as u32 * 299 + pixel[1] as u32 * 587 + pixel[2] as u32 * 114) / 1000) as u8;
        let val = if brightness > cutoff { 255u8 } else { 0u8 };
        pixel[0] = val;
        pixel[1] = val;
        pixel[2] = val;
    }

    // Step 2: Blur → re-threshold for smooth edges.
    // Blur softens the jagged pixel staircases into gradients.
    // Re-threshold snaps back to pure B&W along the smooth contour.
    let smoothing = config.edge_smoothing;
    if smoothing > 0.01 {
        let centers = [[0u8, 0, 0], [255u8, 255, 255]];
        blur_then_threshold(rgba, &centers, smoothing);
    }
}

/// Blur the image, then re-threshold to nearest palette color.
/// Light Gaussian pre-blur for non-Logo modes.
/// Smooths jagged pixel boundaries before VTracer traces them.
/// Unlike blur_then_threshold, this does NOT re-snap to a palette —
/// it just softens edges so VTracer produces smoother curves.
/// Boost luminance contrast in achromatic (low-saturation) regions.
///
/// VTracer's `layer_difference` creates uniform color steps. In saturated
/// areas (pinks, reds), variation spans L+a+b → many layers. In achromatic
/// areas (white-to-gray), variation is only in L → fewer layers → visible banding.
///
/// This function stretches the luminance range of desaturated pixels so
/// VTracer "sees" bigger differences and allocates more gradient layers.
fn boost_achromatic_contrast(rgba: &mut RgbaImage) {
    // Find luminance range of achromatic pixels
    let mut achromatic_lums: Vec<u8> = Vec::new();

    for pixel in rgba.pixels() {
        let r = pixel[0] as f32;
        let g = pixel[1] as f32;
        let b = pixel[2] as f32;

        // Saturation check: max-min channel difference
        let max_c = r.max(g).max(b);
        let min_c = r.min(g).min(b);
        let sat = if max_c > 0.0 { (max_c - min_c) / max_c } else { 0.0 };

        // Only process low-saturation pixels (grays, whites, near-whites)
        if sat < 0.15 {
            let lum = (0.299 * r + 0.587 * g + 0.114 * b) as u8;
            achromatic_lums.push(lum);
        }
    }

    if achromatic_lums.len() < 100 {
        return; // Not enough achromatic pixels to matter
    }

    // Find the luminance range of achromatic pixels
    let lum_min = *achromatic_lums.iter().min().unwrap() as f32;
    let lum_max = *achromatic_lums.iter().max().unwrap() as f32;
    let lum_range = lum_max - lum_min;

    if lum_range < 20.0 {
        return; // Range too small to stretch meaningfully
    }

    // Stretch factor: expand the achromatic luminance range by ~40%
    // This makes subtle gray differences more visible to VTracer.
    let stretch = 1.4_f32;
    let mid = (lum_min + lum_max) / 2.0;

    let mut count = 0u32;
    for pixel in rgba.pixels_mut() {
        let r = pixel[0] as f32;
        let g = pixel[1] as f32;
        let b = pixel[2] as f32;

        let max_c = r.max(g).max(b);
        let min_c = r.min(g).min(b);
        let sat = if max_c > 0.0 { (max_c - min_c) / max_c } else { 0.0 };

        if sat < 0.15 {
            // Stretch luminance away from midpoint
            let lum = 0.299 * r + 0.587 * g + 0.114 * b;
            let new_lum = mid + (lum - mid) * stretch;
            let scale = if lum > 0.5 { new_lum / lum } else { 1.0 };

            pixel[0] = (r * scale).clamp(0.0, 255.0) as u8;
            pixel[1] = (g * scale).clamp(0.0, 255.0) as u8;
            pixel[2] = (b * scale).clamp(0.0, 255.0) as u8;
            count += 1;
        }
    }

    if count > 0 {
        tracing::info!(
            "Achromatic contrast boost: stretched {} pixels (lum range {:.0}-{:.0}, factor {:.1}x)",
            count, lum_min, lum_max, stretch
        );
    }
}

fn pre_blur(rgba: &mut RgbaImage, radius: f64) {
    let width = rgba.width() as usize;
    let height = rgba.height() as usize;

    let kernel_radius = (radius * 2.5).ceil() as i32;
    let sigma = radius.max(0.3);

    let mut kernel = Vec::new();
    let mut total = 0.0_f64;
    for i in -kernel_radius..=kernel_radius {
        let w = (-(i as f64).powi(2) / (2.0 * sigma * sigma)).exp();
        kernel.push(w);
        total += w;
    }
    let kernel: Vec<f64> = kernel.iter().map(|w| w / total).collect();

    // Horizontal pass
    let mut temp = rgba.clone();
    for y in 0..height {
        for x in 0..width {
            let mut r = 0.0_f64;
            let mut g = 0.0_f64;
            let mut b = 0.0_f64;
            for (ki, &w) in kernel.iter().enumerate() {
                let sx = (x as i32 + ki as i32 - kernel_radius).clamp(0, width as i32 - 1) as u32;
                let p = rgba.get_pixel(sx, y as u32);
                r += p[0] as f64 * w;
                g += p[1] as f64 * w;
                b += p[2] as f64 * w;
            }
            let p = temp.get_pixel_mut(x as u32, y as u32);
            p[0] = r.round().clamp(0.0, 255.0) as u8;
            p[1] = g.round().clamp(0.0, 255.0) as u8;
            p[2] = b.round().clamp(0.0, 255.0) as u8;
        }
    }

    // Vertical pass
    for y in 0..height {
        for x in 0..width {
            let mut r = 0.0_f64;
            let mut g = 0.0_f64;
            let mut b = 0.0_f64;
            for (ki, &w) in kernel.iter().enumerate() {
                let sy = (y as i32 + ki as i32 - kernel_radius).clamp(0, height as i32 - 1) as u32;
                let p = temp.get_pixel(x as u32, sy);
                r += p[0] as f64 * w;
                g += p[1] as f64 * w;
                b += p[2] as f64 * w;
            }
            let p = rgba.get_pixel_mut(x as u32, y as u32);
            p[0] = r.round().clamp(0.0, 255.0) as u8;
            p[1] = g.round().clamp(0.0, 255.0) as u8;
            p[2] = b.round().clamp(0.0, 255.0) as u8;
        }
    }

    tracing::info!("Edge smoothing pre-blur: radius={:.1}px", radius);
}

/// The blur creates smooth gradients at boundaries; the threshold
/// snaps them back to hard colors along the smoothed contour.
fn blur_then_threshold(rgba: &mut RgbaImage, centers: &[[u8; 3]], radius: f64) {
    let width = rgba.width() as usize;
    let height = rgba.height() as usize;

    // Gaussian blur — separable (horizontal then vertical) for speed.
    let kernel_radius = (radius * 2.5).ceil() as i32;
    let sigma = radius.max(0.3);

    // Build 1D kernel
    let mut kernel = Vec::new();
    let mut total = 0.0_f64;
    for i in -kernel_radius..=kernel_radius {
        let w = (-(i as f64).powi(2) / (2.0 * sigma * sigma)).exp();
        kernel.push(w);
        total += w;
    }
    let kernel: Vec<f64> = kernel.iter().map(|w| w / total).collect();

    // Horizontal pass
    let mut temp = rgba.clone();
    for y in 0..height {
        for x in 0..width {
            let mut r = 0.0_f64;
            let mut g = 0.0_f64;
            let mut b = 0.0_f64;
            for (ki, &w) in kernel.iter().enumerate() {
                let sx = (x as i32 + ki as i32 - kernel_radius).clamp(0, width as i32 - 1) as u32;
                let p = rgba.get_pixel(sx, y as u32);
                r += p[0] as f64 * w;
                g += p[1] as f64 * w;
                b += p[2] as f64 * w;
            }
            let p = temp.get_pixel_mut(x as u32, y as u32);
            p[0] = r.round().clamp(0.0, 255.0) as u8;
            p[1] = g.round().clamp(0.0, 255.0) as u8;
            p[2] = b.round().clamp(0.0, 255.0) as u8;
        }
    }

    // Vertical pass
    let mut blurred = temp.clone();
    for y in 0..height {
        for x in 0..width {
            let mut r = 0.0_f64;
            let mut g = 0.0_f64;
            let mut b = 0.0_f64;
            for (ki, &w) in kernel.iter().enumerate() {
                let sy = (y as i32 + ki as i32 - kernel_radius).clamp(0, height as i32 - 1) as u32;
                let p = temp.get_pixel(x as u32, sy);
                r += p[0] as f64 * w;
                g += p[1] as f64 * w;
                b += p[2] as f64 * w;
            }
            let p = blurred.get_pixel_mut(x as u32, y as u32);
            p[0] = r.round().clamp(0.0, 255.0) as u8;
            p[1] = g.round().clamp(0.0, 255.0) as u8;
            p[2] = b.round().clamp(0.0, 255.0) as u8;
        }
    }

    // Re-threshold: snap every blurred pixel back to nearest palette color.
    // The boundary now follows the smooth blur contour.
    for (i, pixel) in rgba.pixels_mut().enumerate() {
        let bp = blurred.get_pixel(i as u32 % blurred.width(), i as u32 / blurred.width());
        let px = [bp[0], bp[1], bp[2]];
        let mut best_dist = u32::MAX;
        let mut best_center = centers[0];
        for &c in centers {
            let dr = px[0] as i32 - c[0] as i32;
            let dg = px[1] as i32 - c[1] as i32;
            let db = px[2] as i32 - c[2] as i32;
            let dist = (dr * dr + dg * dg + db * db) as u32;
            if dist < best_dist {
                best_dist = dist;
                best_center = c;
            }
        }
        pixel[0] = best_center[0];
        pixel[1] = best_center[1];
        pixel[2] = best_center[2];
    }

    tracing::info!("Logo blur→threshold: radius={:.1}px, {} palette colors", radius, centers.len());
}

/// Fast k-means in RGB space for thresholding.
fn rgb_kmeans(pixels: &[[u8; 3]], k: usize, iterations: usize) -> Vec<[u8; 3]> {
    if pixels.is_empty() || k == 0 { return vec![[128, 128, 128]]; }

    // Initialize centers by sampling evenly through the pixel array
    let step = (pixels.len() / k).max(1);
    let mut centers: Vec<[f64; 3]> = (0..k)
        .map(|i| {
            let p = pixels[(i * step) % pixels.len()];
            [p[0] as f64, p[1] as f64, p[2] as f64]
        })
        .collect();

    // Subsample for speed — use at most 20k pixels
    let sample_step = (pixels.len() / 20_000).max(1);

    for _ in 0..iterations {
        let mut sums = vec![[0.0_f64; 3]; k];
        let mut counts = vec![0u64; k];

        for (idx, px) in pixels.iter().enumerate() {
            if idx % sample_step != 0 { continue; }
            let pr = px[0] as f64;
            let pg = px[1] as f64;
            let pb = px[2] as f64;

            let mut best = 0;
            let mut best_dist = f64::MAX;
            for (j, c) in centers.iter().enumerate() {
                let d = (pr - c[0]).powi(2) + (pg - c[1]).powi(2) + (pb - c[2]).powi(2);
                if d < best_dist {
                    best_dist = d;
                    best = j;
                }
            }

            sums[best][0] += pr;
            sums[best][1] += pg;
            sums[best][2] += pb;
            counts[best] += 1;
        }

        for j in 0..k {
            if counts[j] > 0 {
                centers[j] = [
                    sums[j][0] / counts[j] as f64,
                    sums[j][1] / counts[j] as f64,
                    sums[j][2] / counts[j] as f64,
                ];
            }
        }
    }

    centers.iter().map(|c| [c[0].round() as u8, c[1].round() as u8, c[2].round() as u8]).collect()
}

/// Smooth only the boundary pixels between different-colored regions.
/// This creates clean anti-aliased edges without blurring the flat interior.
///
/// `radius`: smoothing radius in pixels (1.0 = subtle, 3.0 = very smooth).
fn smooth_color_boundaries(rgba: &mut RgbaImage, width: usize, height: usize, radius: f64) {
    let w = width;
    let h = height;

    // Step 1: Find boundary pixels (pixels adjacent to a different color)
    let pixels: Vec<[u8; 3]> = rgba.pixels().map(|p| [p[0], p[1], p[2]]).collect();
    let mut is_boundary = vec![false; w * h];

    for y in 1..h - 1 {
        for x in 1..w - 1 {
            let idx = y * w + x;
            let center = pixels[idx];
            // Check 4-neighbors
            let neighbors = [
                pixels[idx - 1], pixels[idx + 1],
                pixels[idx - w], pixels[idx + w],
            ];
            for n in &neighbors {
                if *n != center {
                    is_boundary[idx] = true;
                    break;
                }
            }
        }
    }

    // Step 2: For boundary pixels and their immediate neighbors,
    // apply a small Gaussian blur
    let kernel_size = (radius * 2.0).ceil() as i32 + 1;
    let sigma = radius.max(0.5);

    // Build Gaussian kernel weights
    let mut kernel = Vec::new();
    let mut total_weight = 0.0;
    for dy in -kernel_size..=kernel_size {
        for dx in -kernel_size..=kernel_size {
            let d2 = (dx * dx + dy * dy) as f64;
            let w = (-d2 / (2.0 * sigma * sigma)).exp();
            kernel.push((dx, dy, w));
            total_weight += w;
        }
    }
    // Normalize
    let kernel: Vec<(i32, i32, f64)> = kernel.iter()
        .map(|&(dx, dy, w)| (dx, dy, w / total_weight))
        .collect();

    // Step 3: Apply blur only to boundary region (boundary + 1px neighbors)
    let mut boundary_expanded = is_boundary.clone();
    for y in 1..h - 1 {
        for x in 1..w - 1 {
            let idx = y * w + x;
            if is_boundary[idx] {
                // Also mark immediate neighbors for smoothing
                boundary_expanded[idx - 1] = true;
                boundary_expanded[idx + 1] = true;
                boundary_expanded[idx - w] = true;
                boundary_expanded[idx + w] = true;
            }
        }
    }

    let mut result = rgba.clone();
    for y in 0..h {
        for x in 0..w {
            let idx = y * w + x;
            if !boundary_expanded[idx] { continue; }

            let mut r = 0.0_f64;
            let mut g = 0.0_f64;
            let mut b = 0.0_f64;

            for &(dx, dy, w) in &kernel {
                let nx = x as i32 + dx;
                let ny = y as i32 + dy;
                if nx < 0 || ny < 0 || nx >= width as i32 || ny >= height as i32 { continue; }
                let ni = ny as usize * width + nx as usize;
                r += pixels[ni][0] as f64 * w;
                g += pixels[ni][1] as f64 * w;
                b += pixels[ni][2] as f64 * w;
            }

            let p = result.get_pixel_mut(x as u32, y as u32);
            p[0] = r.round().clamp(0.0, 255.0) as u8;
            p[1] = g.round().clamp(0.0, 255.0) as u8;
            p[2] = b.round().clamp(0.0, 255.0) as u8;
        }
    }

    *rgba = result;
    tracing::info!("Logo edge smoothing: radius={:.1}px", radius);
}

// ── JPEG artifact cleanup ───────────────────────────────────────────

/// For bimodal images (e.g. black logos on solid background), apply a
/// brightness threshold to snap every pixel cleanly to either "dark"
/// or "light". This eliminates the gray anti-aliasing fringe from JPEG.
/// Returns true if cleanup was applied.
fn clean_jpeg_artifacts(rgba: &mut RgbaImage) -> bool {
    // Check if image is bimodal: build brightness histogram
    let pixel_count = (rgba.width() * rgba.height()) as usize;
    if pixel_count < 100 { return false; }

    let step = (pixel_count / 3000).max(1);
    let mut histogram = [0u32; 256];

    for (i, pixel) in rgba.pixels().enumerate() {
        if i % step != 0 { continue; }
        let brightness = ((pixel[0] as u32 * 299 + pixel[1] as u32 * 587 + pixel[2] as u32 * 114) / 1000) as u8;
        histogram[brightness as usize] += 1;
    }

    let total: u32 = histogram.iter().sum();
    if total == 0 { return false; }

    // Find peaks: look for two clusters — one dark, one light.
    // Split the histogram at brightness 128. If >40% of pixels are in
    // the bottom quarter (0-64) and >40% in the top quarter (192-255),
    // the image is bimodal.
    let dark: u32 = histogram[..80].iter().sum();
    let light: u32 = histogram[160..].iter().sum();
    let dark_pct = dark as f64 / total as f64;
    let light_pct = light as f64 / total as f64;
    let middle_pct = 1.0 - dark_pct - light_pct;

    tracing::debug!(
        "JPEG cleanup check: dark={:.0}% middle={:.0}% light={:.0}%",
        dark_pct * 100.0, middle_pct * 100.0, light_pct * 100.0
    );

    // Bimodal: BOTH dark and light must be substantial (>15% each),
    // and the middle zone must be small (<15%).
    // This catches "black logos on green" but NOT "anatomy with white bg"
    // (which has lots of light but little dark).
    if middle_pct > 0.15 || dark_pct < 0.15 || light_pct < 0.15 {
        return false;
    }

    tracing::info!("JPEG cleanup: applying brightness threshold for bimodal image");

    // Find the optimal threshold using Otsu's method (simplified).
    // The threshold is where the histogram valley is between the two peaks.
    let threshold = find_otsu_threshold(&histogram, total);

    // Find the average color of the light cluster (background)
    let mut bg_r: u64 = 0;
    let mut bg_g: u64 = 0;
    let mut bg_b: u64 = 0;
    let mut bg_count: u64 = 0;

    // Find the average color of the dark cluster (foreground)
    let mut fg_r: u64 = 0;
    let mut fg_g: u64 = 0;
    let mut fg_b: u64 = 0;
    let mut fg_count: u64 = 0;

    for (i, pixel) in rgba.pixels().enumerate() {
        if i % step != 0 { continue; }
        let brightness = ((pixel[0] as u32 * 299 + pixel[1] as u32 * 587 + pixel[2] as u32 * 114) / 1000) as u8;
        if brightness > threshold {
            bg_r += pixel[0] as u64;
            bg_g += pixel[1] as u64;
            bg_b += pixel[2] as u64;
            bg_count += 1;
        } else {
            fg_r += pixel[0] as u64;
            fg_g += pixel[1] as u64;
            fg_b += pixel[2] as u64;
            fg_count += 1;
        }
    }

    if bg_count == 0 || fg_count == 0 { return false; }

    let bg = [
        (bg_r / bg_count) as u8,
        (bg_g / bg_count) as u8,
        (bg_b / bg_count) as u8,
    ];
    let fg = [
        (fg_r / fg_count) as u8,
        (fg_g / fg_count) as u8,
        (fg_b / fg_count) as u8,
    ];

    tracing::debug!(
        "JPEG cleanup: threshold={}, bg=rgb({},{},{}), fg=rgb({},{},{})",
        threshold, bg[0], bg[1], bg[2], fg[0], fg[1], fg[2]
    );

    // Snap every pixel to either fg or bg based on brightness
    for pixel in rgba.pixels_mut() {
        let brightness = ((pixel[0] as u32 * 299 + pixel[1] as u32 * 587 + pixel[2] as u32 * 114) / 1000) as u8;
        if brightness > threshold {
            pixel[0] = bg[0];
            pixel[1] = bg[1];
            pixel[2] = bg[2];
        } else {
            pixel[0] = fg[0];
            pixel[1] = fg[1];
            pixel[2] = fg[2];
        }
    }

    true
}

/// Simple Otsu's threshold: find the brightness value that minimizes
/// the combined within-class variance.
pub(crate) fn find_otsu_threshold(histogram: &[u32; 256], total: u32) -> u8 {
    let mut best_threshold = 128u8;
    let mut best_variance = f64::MAX;

    let mut w0: u32 = 0;
    let mut sum0: u64 = 0;
    let total_sum: u64 = histogram.iter().enumerate().map(|(i, &c)| i as u64 * c as u64).sum();

    for t in 0..255u8 {
        w0 += histogram[t as usize];
        let w1 = total - w0;
        if w0 == 0 || w1 == 0 { continue; }

        sum0 += t as u64 * histogram[t as usize] as u64;
        let sum1 = total_sum - sum0;

        let mean0 = sum0 as f64 / w0 as f64;
        let mean1 = sum1 as f64 / w1 as f64;

        let between_variance = w0 as f64 * w1 as f64 * (mean0 - mean1) * (mean0 - mean1);
        // We want to MAXIMIZE between-class variance (equivalent to minimizing within-class)
        let neg_variance = -between_variance;

        if neg_variance < best_variance {
            best_variance = neg_variance;
            best_threshold = t;
        }
    }

    best_threshold
}

// ── Edge-based background separation ────────────────────────────────

/// Separate subject from background using color-based flood fill from
/// the image corners. If all 4 corners share a similar color, flood-fill
/// inward from the edges, only expanding to pixels that are close in color
/// to the corner color. Stops at the subject boundary.
fn separate_background(
    rgba: &RgbaImage,
    existing_mask: Option<Vec<bool>>,
    width: u32,
    height: u32,
) -> Option<Vec<bool>> {
    if existing_mask.is_some() {
        return existing_mask;
    }

    let w = width as usize;
    let h = height as usize;
    let pixel_count = w * h;
    if w < 20 || h < 20 { return None; }

    // Sample all 4 corners (5x5 block each) to find the background color.
    let mut corner_colors: Vec<[u8; 3]> = Vec::new();
    for &(cx, cy) in &[(2, 2), (w - 3, 2), (2, h - 3), (w - 3, h - 3)] {
        for dy in 0..5usize {
            for dx in 0..5usize {
                let x = (cx + dx).min(w - 1);
                let y = (cy + dy).min(h - 1);
                let p = rgba.get_pixel(x as u32, y as u32);
                corner_colors.push([p[0], p[1], p[2]]);
            }
        }
    }

    let bg_color = most_common_color(&corner_colors);

    // Check that all 4 corners agree on the background color
    let matching = corner_colors
        .iter()
        .filter(|&&c| color_distance_rgb(c, bg_color) <= 20)
        .count();
    let corner_agreement = matching as f64 / corner_colors.len() as f64;

    // Don't remove white/near-white backgrounds — they're the SVG default
    // and removing them causes the post-process rect to pick a wrong color.
    let bg_brightness = (bg_color[0] as u32 + bg_color[1] as u32 + bg_color[2] as u32) / 3;
    if bg_brightness > 220 {
        tracing::debug!(
            "Background separation: skipping white/light background (brightness {})",
            bg_brightness
        );
        return None;
    }

    if corner_agreement < 0.80 {
        tracing::debug!(
            "Background separation: corners don't agree ({:.0}% match rgb({},{},{}))",
            corner_agreement * 100.0, bg_color[0], bg_color[1], bg_color[2]
        );
        return None;
    }

    tracing::debug!(
        "Background separation: corner color rgb({},{},{}) — {:.0}% agreement",
        bg_color[0], bg_color[1], bg_color[2], corner_agreement * 100.0
    );

    // Adaptive flood fill from border pixels.
    // Uses two strategies:
    //  1. Color-neighbor flood fill: follows gradual color changes from borders
    //  2. Checkerboard detection: identifies alternating 2-tone grid patterns
    //     (common in 3D renders with baked transparency grids)
    let seed_tolerance = 15u16;
    let neighbor_tolerance = 6u16;

    // Store the color of each pixel for neighbor comparisons
    let pixel_colors: Vec<[u8; 3]> = rgba
        .pixels()
        .map(|p| [p[0], p[1], p[2]])
        .collect();

    // Detect checkerboard pattern: sample a grid and look for two alternating
    // neutral colors. If found, both colors are treated as background.
    let checker_colors = detect_checkerboard(&pixel_colors, w, h, bg_color);

    let mut is_background = vec![false; pixel_count];
    let mut queue: std::collections::VecDeque<(usize, usize)> = std::collections::VecDeque::new();

    // Seed border pixels that match background color(s)
    for x in 0..w {
        for &y in &[0, h - 1] {
            let idx = y * w + x;
            if is_bg_color(&pixel_colors[idx], bg_color, seed_tolerance, &checker_colors) {
                is_background[idx] = true;
                queue.push_back((x, y));
            }
        }
    }
    for y in 1..h - 1 {
        for &x in &[0, w - 1] {
            let idx = y * w + x;
            if is_bg_color(&pixel_colors[idx], bg_color, seed_tolerance, &checker_colors) {
                is_background[idx] = true;
                queue.push_back((x, y));
            }
        }
    }

    // Compute the background color's own saturation so we can allow
    // similar-saturation pixels. Dark tinted backgrounds (e.g., dark purple)
    // have higher saturation than neutral greys but are still "background".
    let bg_max = bg_color[0].max(bg_color[1]).max(bg_color[2]);
    let bg_min = bg_color[0].min(bg_color[1]).min(bg_color[2]);
    let bg_saturation = bg_max - bg_min;
    // Allow pixels with saturation up to bg_saturation + 10 (or minimum 12)
    let max_saturation = (bg_saturation + 10).max(12);

    while let Some((x, y)) = queue.pop_front() {
        let parent_idx = y * w + x;
        let parent_color = pixel_colors[parent_idx];

        for (nx, ny) in neighbors_4(x, y, w, h) {
            let nidx = ny * w + nx;
            if is_background[nidx] { continue; }

            let neighbor_color = pixel_colors[nidx];

            // Saturation check: allow pixels with similar saturation to the
            // detected background. This handles tinted backgrounds (dark purple,
            // warm grey) that aren't perfectly neutral.
            let max_ch = neighbor_color[0].max(neighbor_color[1]).max(neighbor_color[2]);
            let min_ch = neighbor_color[0].min(neighbor_color[1]).min(neighbor_color[2]);
            let saturation = max_ch - min_ch;
            let sat_ok = saturation <= max_saturation;

            // Must be close to parent AND close to the original bg color
            // (prevents gradient drift from bg into subject).
            let close_to_parent = color_distance_rgb(neighbor_color, parent_color) <= neighbor_tolerance;
            let close_to_bg = color_distance_rgb(neighbor_color, bg_color) <= seed_tolerance;
            let matches_checker = is_bg_color(&neighbor_color, bg_color, seed_tolerance, &checker_colors);

            if (close_to_parent && close_to_bg && sat_ok) || (matches_checker && sat_ok) {
                is_background[nidx] = true;
                queue.push_back((nx, ny));
            }
        }
    }

    let bg_count = is_background.iter().filter(|&&b| b).count();
    let bg_pct = bg_count as f64 / pixel_count as f64 * 100.0;

    tracing::debug!("Background separation: {:.1}% classified as background", bg_pct);

    if bg_pct < 5.0 || bg_pct > 95.0 {
        tracing::debug!("Background separation: rejected ({:.1}% outside safe range)", bg_pct);
        return None;
    }

    tracing::info!("Background separation: removed {:.1}% background pixels", bg_pct);

    let mask: Vec<bool> = is_background.iter().map(|&b| !b).collect();
    Some(mask)
}

// ── Helpers ─────────────────────────────────────────────────────────

/// Check if a pixel color matches the background (either the primary bg color
/// or one of the detected checkerboard colors).
fn is_bg_color(
    color: &[u8; 3],
    bg_color: [u8; 3],
    tolerance: u16,
    checker_colors: &Option<([u8; 3], [u8; 3])>,
) -> bool {
    if color_distance_rgb(*color, bg_color) <= tolerance {
        return true;
    }
    if let Some((c1, c2)) = checker_colors {
        if color_distance_rgb(*color, *c1) <= tolerance
            || color_distance_rgb(*color, *c2) <= tolerance
        {
            return true;
        }
    }
    false
}

/// Detect a checkerboard transparency grid pattern.
/// Looks for two alternating neutral colors in a grid pattern near the corners.
/// Returns Some((color_a, color_b)) if a checkerboard is detected.
fn detect_checkerboard(
    pixel_colors: &[[u8; 3]],
    w: usize,
    h: usize,
    bg_color: [u8; 3],
) -> Option<([u8; 3], [u8; 3])> {
    // Sample a region near the top-left corner where we expect pure background.
    // Look for an alternating pattern of two neutral colors.
    let sample_size = 20.min(w / 2).min(h / 2);
    if sample_size < 8 {
        return None;
    }

    // Collect unique neutral colors in the sample region
    let mut color_counts: std::collections::HashMap<[u8; 3], u32> = std::collections::HashMap::new();
    for y in 0..sample_size {
        for x in 0..sample_size {
            let idx = y * w + x;
            let c = pixel_colors[idx];
            let max_ch = c[0].max(c[1]).max(c[2]);
            let min_ch = c[0].min(c[1]).min(c[2]);
            if max_ch - min_ch <= 8 {
                // Neutral color — quantize to reduce noise
                let quantized = [c[0] / 4 * 4, c[1] / 4 * 4, c[2] / 4 * 4];
                *color_counts.entry(quantized).or_insert(0) += 1;
            }
        }
    }

    // Need at least 2 distinct colors each covering >20% of the sample
    let total_neutral: u32 = color_counts.values().sum();
    if total_neutral < (sample_size * sample_size / 2) as u32 {
        return None; // Less than 50% neutral — not a checker pattern
    }

    let mut sorted: Vec<_> = color_counts.into_iter().collect();
    sorted.sort_by(|a, b| b.1.cmp(&a.1));

    if sorted.len() < 2 {
        return None;
    }

    let (color_a, count_a) = sorted[0];
    let (color_b, count_b) = sorted[1];

    let pct_a = count_a as f64 / total_neutral as f64;
    let pct_b = count_b as f64 / total_neutral as f64;

    // Both colors must be substantial (>20%) and their ratio should be roughly balanced (<3:1)
    if pct_a < 0.20 || pct_b < 0.20 {
        return None;
    }
    if count_a > count_b * 3 {
        return None;
    }

    // Verify alternating pattern: check if neighboring pixels alternate between the two colors
    let mut alternating_count = 0u32;
    let mut checked = 0u32;
    for y in 0..sample_size.min(10) {
        for x in 0..sample_size.min(10) - 1 {
            let idx = y * w + x;
            let idx_right = y * w + x + 1;
            let c = pixel_colors[idx];
            let cr = pixel_colors[idx_right];
            let c_near_a = color_distance_rgb(c, color_a) <= 15;
            let c_near_b = color_distance_rgb(c, color_b) <= 15;
            let cr_near_a = color_distance_rgb(cr, color_a) <= 15;
            let cr_near_b = color_distance_rgb(cr, color_b) <= 15;
            if (c_near_a || c_near_b) && (cr_near_a || cr_near_b) {
                checked += 1;
                // Alternating: left is A and right is B, or vice versa
                if (c_near_a && cr_near_b) || (c_near_b && cr_near_a) {
                    alternating_count += 1;
                }
            }
        }
    }

    if checked < 20 || (alternating_count as f64 / checked as f64) < 0.3 {
        // Not enough alternation — might be a gradient, not a checker
        // Use a looser check: just two distinct neutral colors near bg_color
        let a_near_bg = color_distance_rgb(color_a, bg_color) <= 30;
        let b_near_bg = color_distance_rgb(color_b, bg_color) <= 30;
        if a_near_bg && b_near_bg {
            tracing::info!(
                "Background: detected dual-tone background rgb({},{},{}) + rgb({},{},{})",
                color_a[0], color_a[1], color_a[2],
                color_b[0], color_b[1], color_b[2],
            );
            return Some((color_a, color_b));
        }
        return None;
    }

    tracing::info!(
        "Background: detected checkerboard pattern rgb({},{},{}) + rgb({},{},{}) ({:.0}% alternating)",
        color_a[0], color_a[1], color_a[2],
        color_b[0], color_b[1], color_b[2],
        100.0 * alternating_count as f64 / checked as f64,
    );
    Some((color_a, color_b))
}

fn neighbors_4(x: usize, y: usize, w: usize, h: usize) -> Vec<(usize, usize)> {
    let mut n = Vec::with_capacity(4);
    if x > 0 { n.push((x - 1, y)); }
    if x + 1 < w { n.push((x + 1, y)); }
    if y > 0 { n.push((x, y - 1)); }
    if y + 1 < h { n.push((x, y + 1)); }
    n
}

fn most_common_color(colors: &[[u8; 3]]) -> [u8; 3] {
    use std::collections::HashMap;
    let mut counts: HashMap<[u8; 3], usize> = HashMap::new();
    for &c in colors {
        let key = [c[0] & 0xF8, c[1] & 0xF8, c[2] & 0xF8];
        *counts.entry(key).or_insert(0) += 1;
    }
    counts.into_iter().max_by_key(|&(_, count)| count)
        .map(|(k, _)| [k[0] | 4, k[1] | 4, k[2] | 4])
        .unwrap_or([0, 0, 0])
}

fn color_distance_rgb(a: [u8; 3], b: [u8; 3]) -> u16 {
    let dr = (a[0] as i16 - b[0] as i16).unsigned_abs();
    let dg = (a[1] as i16 - b[1] as i16).unsigned_abs();
    let db = (a[2] as i16 - b[2] as i16).unsigned_abs();
    dr.max(dg).max(db)
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::{DynamicImage, RgbaImage};

    #[test]
    fn transparent_pixels_produce_mask() {
        let mut img = RgbaImage::new(2, 2);
        img.put_pixel(0, 0, image::Rgba([255, 0, 0, 255]));
        img.put_pixel(1, 0, image::Rgba([0, 0, 0, 0]));
        img.put_pixel(0, 1, image::Rgba([0, 255, 0, 128]));
        img.put_pixel(1, 1, image::Rgba([0, 0, 255, 255]));

        let dynamic = DynamicImage::ImageRgba8(img);
        let config = VectorizeConfig::default();
        let prepared = prepare(&dynamic, &config);
        let mask = prepared.opaque_mask.as_ref().expect("mask should be Some");
        assert_eq!(mask.len(), 4);
        assert!(mask[0]);
        assert!(!mask[1]);
        assert!(mask[2]);
        assert!(mask[3]);
    }

    #[test]
    fn fully_opaque_image_has_no_mask_if_no_edges() {
        // Uniform color image — no edges to separate on
        let mut img = RgbaImage::new(10, 10);
        for pixel in img.pixels_mut() {
            *pixel = image::Rgba([128, 128, 128, 255]);
        }
        let dynamic = DynamicImage::ImageRgba8(img);
        let config = VectorizeConfig::default();
        let prepared = prepare(&dynamic, &config);
        // No strong edges → background separation rejected
        assert!(prepared.opaque_mask.is_none());
    }

    #[test]
    fn otsu_threshold_works() {
        let mut hist = [0u32; 256];
        // Two peaks: one at 50, one at 200
        for i in 40..60 { hist[i] = 100; }
        for i in 190..210 { hist[i] = 100; }
        let total: u32 = hist.iter().sum();
        let threshold = find_otsu_threshold(&hist, total);
        // Threshold should be between the two peaks
        // Threshold should be between the two peaks (59-190 is valid —
        // 59 is the last bin of peak 1, which correctly separates the two groups).
        assert!(threshold >= 59 && threshold < 190, "threshold={}", threshold);
    }
}
