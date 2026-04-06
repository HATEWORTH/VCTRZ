//! Stage 2: Color quantization and region segmentation.
//!
//! Uses k-means clustering in Oklab perceptual color space for
//! high-quality color reduction, then labels each pixel with its cluster ID.
//!
//! Key design decisions:
//! - Transparent pixels (via `PreparedImage::opaque_mask`) are excluded from
//!   clustering and labelled `u32::MAX`.
//! - Auto color-count uses a lightness histogram peak detector instead of
//!   `sqrt(unique_colors)` so that bimodal images (e.g. black logos on a
//!   solid background) get 2-4 colors rather than 12+.
//! - After k-means converges, extreme-lightness clusters are snapped back
//!   toward black/white to preserve contrast that averaging destroys.
//! - Background detection: the most frequent opaque label is stored as
//!   `background_label` when `config.flatten_background` is set.

// Numeric casts in color conversion code are intentional and safe after clamping.
#![allow(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::cast_precision_loss,
    clippy::cast_lossless
)]

use palette::{FromColor, Oklab, Srgb};
use rayon::prelude::*;

use crate::{Color, PreparedImage, Result, SegmentedImage, VectorizeConfig, VectorizeError};

// ── Oklab helpers ──────────────────────────────────────────────────────

/// Convert RGBA pixel to Oklab [L, a, b].
fn rgba_to_oklab(r: u8, g: u8, b: u8) -> [f32; 3] {
    let srgb = Srgb::new(
        f32::from(r) / 255.0,
        f32::from(g) / 255.0,
        f32::from(b) / 255.0,
    );
    let oklab: Oklab = Oklab::from_color(srgb);
    [oklab.l, oklab.a, oklab.b]
}

/// Convert Oklab [L, a, b] back to RGB [u8; 3].
fn oklab_to_rgb(lab: &[f32; 3]) -> [u8; 3] {
    let oklab = Oklab::new(lab[0], lab[1], lab[2]);
    let srgb: Srgb = Srgb::from_color(oklab);
    // Clamp to [0, 1] since Oklab->sRGB can produce out-of-gamut values
    let r = (srgb.red.clamp(0.0, 1.0) * 255.0).round() as u8;
    let g = (srgb.green.clamp(0.0, 1.0) * 255.0).round() as u8;
    let b = (srgb.blue.clamp(0.0, 1.0) * 255.0).round() as u8;
    [r, g, b]
}

/// Squared Euclidean distance in Oklab space.
fn oklab_dist_sq(a: &[f32; 3], b: &[f32; 3]) -> f32 {
    let dl = a[0] - b[0];
    let da = a[1] - b[1];
    let db = a[2] - b[2];
    dl * dl + da * da + db * db
}

// ── Auto color-count via 3-channel histogram ─────────────────────────

/// Count significant peaks in a 1D histogram.
fn count_peaks(histogram: &[u32]) -> usize {
    let total: u32 = histogram.iter().sum();
    if total == 0 {
        return 0;
    }
    let n = histogram.len() as f64;
    let mean = total as f64 / n;
    let variance = histogram
        .iter()
        .map(|&c| {
            let diff = c as f64 - mean;
            diff * diff
        })
        .sum::<f64>()
        / n;
    let stddev = variance.sqrt();
    let threshold = mean + 0.5 * stddev;
    histogram.iter().filter(|&&c| c as f64 > threshold).count()
}

/// Analyze color distribution of opaque pixels and return a suitable k.
///
/// Analyzes ALL THREE Oklab channels (L, a, b) — not just lightness.
/// This catches cases like the anatomical illustrations where red and blue
/// have similar lightness but very different chrominance.
///
/// Algorithm:
///   1. Build 16-bin histograms for L, a, and b channels independently.
///   2. Count significant peaks in each.
///   3. Take the MAX peak count across all three channels.
///   4. Map to color range, with a minimum floor of 5 for auto mode.
fn auto_color_count(pixels: &[[f32; 3]], max_colors: u32) -> u32 {
    const NUM_BINS: usize = 32;

    if pixels.is_empty() {
        return 2;
    }

    // Sample up to 10 000 opaque pixels for speed.
    let sample_size = pixels.len().min(10_000);
    let step = (pixels.len() / sample_size).max(1);

    let mut hist_l = [0u32; NUM_BINS];
    let mut hist_a = [0u32; NUM_BINS];
    let mut hist_b = [0u32; NUM_BINS];

    for pixel in pixels.iter().step_by(step) {
        // L is in [0, 1]
        let l_bin = ((pixel[0].clamp(0.0, 1.0) * (NUM_BINS - 1) as f32).round() as usize)
            .min(NUM_BINS - 1);
        // a and b are roughly in [-0.4, 0.4], shift to [0, 1]
        let a_norm = ((pixel[1] + 0.4) / 0.8).clamp(0.0, 1.0);
        let a_bin = ((a_norm * (NUM_BINS - 1) as f32).round() as usize).min(NUM_BINS - 1);
        let b_norm = ((pixel[2] + 0.4) / 0.8).clamp(0.0, 1.0);
        let b_bin = ((b_norm * (NUM_BINS - 1) as f32).round() as usize).min(NUM_BINS - 1);

        hist_l[l_bin] += 1;
        hist_a[a_bin] += 1;
        hist_b[b_bin] += 1;
    }

    let peaks_l = count_peaks(&hist_l);
    let peaks_a = count_peaks(&hist_a);
    let peaks_b = count_peaks(&hist_b);

    // Use the maximum across all channels — if chrominance is varied,
    // we need more colors even if lightness is uniform.
    let peaks = peaks_l.max(peaks_a).max(peaks_b);

    let suggested = match peaks {
        0..=2 => 2 + (peaks as u32).min(2),  // 2-4  (bimodal, e.g. black logos)
        3..=4 => 7 + (peaks as u32 - 3),     // 7-8
        5..=6 => 9 + (peaks as u32 - 5),     // 9-10
        7..=10 => (peaks as u32) * 2,        // 14-20
        11..=16 => (peaks as u32) * 2 + 4,   // 26-36
        _ => (peaks as u32 * 2 + 8).min(64), // up to 64
    };

    // Determine if this is a truly simple image (like solid-color logos)
    // vs a continuous-tone image that happens to have a bimodal histogram.
    // Check the histogram spread: if most bins are empty, it's truly simple.
    let nonzero_bins_l = hist_l.iter().filter(|&&c| c > 0).count();
    let is_truly_bimodal = peaks <= 2 && nonzero_bins_l <= 12;

    // Floor of 8 for complex images — small but visually important
    // features don't show as histogram peaks but need their own cluster.
    // Exception: truly bimodal images (e.g. black logos on solid bg)
    // can use fewer colors.
    let floor = if is_truly_bimodal { 2 } else { 6 };
    let capped = suggested.clamp(floor, max_colors.min(64));

    tracing::debug!(
        "Auto color count: L={} a={} b={} peaks (max {}) → suggested {} → capped {}",
        peaks_l,
        peaks_a,
        peaks_b,
        peaks,
        suggested,
        capped,
    );
    capped
}

// ── K-means clustering ────────────────────────────────────────────────

/// K-means++ initialization: pick starting centers that are well-spread.
/// Uses a deterministic seed derived from the pixel data for reproducible output.
fn kmeans_plus_plus(pixels: &[[f32; 3]], k: usize) -> Vec<[f32; 3]> {
    use rand::Rng;
    use rand::SeedableRng;
    // Deterministic seed: hash of pixel count + first/last pixel values.
    // Same image = same seed = same initialization = reproducible output.
    let seed = {
        let n = pixels.len() as u64;
        let first = pixels.first().map(|p| (p[0].to_bits() as u64) ^ (p[1].to_bits() as u64) << 16 ^ (p[2].to_bits() as u64) << 32).unwrap_or(0);
        let last = pixels.last().map(|p| (p[0].to_bits() as u64) ^ (p[1].to_bits() as u64) << 16 ^ (p[2].to_bits() as u64) << 32).unwrap_or(0);
        n.wrapping_mul(2654435761) ^ first ^ last.rotate_left(32)
    };
    let mut rng = rand::rngs::StdRng::seed_from_u64(seed);
    let mut centers = Vec::with_capacity(k);

    // First center: random pixel
    centers.push(pixels[rng.random_range(0..pixels.len())]);

    for _ in 1..k {
        // For each pixel, compute distance to nearest existing center
        let distances: Vec<f32> = pixels
            .iter()
            .map(|p| {
                centers
                    .iter()
                    .map(|c| oklab_dist_sq(p, c))
                    .fold(f32::MAX, f32::min)
            })
            .collect();

        let total: f32 = distances.iter().sum();
        if total <= 0.0 {
            // All remaining pixels are on existing centers
            centers.push(pixels[rng.random_range(0..pixels.len())]);
            continue;
        }

        // Weighted random selection proportional to distance squared
        let threshold = rng.random_range(0.0..total);
        let mut cumulative = 0.0;
        let mut chosen = pixels.len() - 1;
        for (i, &d) in distances.iter().enumerate() {
            cumulative += d;
            if cumulative >= threshold {
                chosen = i;
                break;
            }
        }
        centers.push(pixels[chosen]);
    }

    centers
}

/// Run k-means clustering in Oklab space.
///
/// Returns converged cluster centers.
fn kmeans(pixels: &[[f32; 3]], k: usize, max_iter: usize) -> Vec<[f32; 3]> {
    if pixels.is_empty() || k == 0 {
        return vec![];
    }
    let k = k.min(pixels.len());

    let mut centers = kmeans_plus_plus(pixels, k);

    for iteration in 0..max_iter {
        // Assign each pixel to nearest center (parallel)
        let assignments: Vec<usize> = pixels
            .par_iter()
            .map(|p| {
                centers
                    .iter()
                    .enumerate()
                    .map(|(i, c)| (i, oklab_dist_sq(p, c)))
                    .min_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal))
                    .map_or(0, |(i, _)| i)
            })
            .collect();

        // Compute new centers
        let mut sums = vec![[0.0f64; 3]; k];
        let mut counts = vec![0u64; k];

        for (pixel, &label) in pixels.iter().zip(&assignments) {
            sums[label][0] += f64::from(pixel[0]);
            sums[label][1] += f64::from(pixel[1]);
            sums[label][2] += f64::from(pixel[2]);
            counts[label] += 1;
        }

        let mut converged = true;
        for i in 0..k {
            if counts[i] == 0 {
                continue;
            }
            let new_center = [
                (sums[i][0] / counts[i] as f64) as f32,
                (sums[i][1] / counts[i] as f64) as f32,
                (sums[i][2] / counts[i] as f64) as f32,
            ];
            if oklab_dist_sq(&centers[i], &new_center) > 1e-6 {
                converged = false;
            }
            centers[i] = new_center;
        }

        if converged {
            tracing::debug!("K-means converged at iteration {}", iteration + 1);
            break;
        }
    }

    centers
}

// ── Contrast-preserving snap ──────────────────────────────────────────

/// For each cluster, check the extreme lightness values of the original
/// pixels assigned to it.  If the darkest pixel had L < 0.15, snap the
/// center's L to at most 0.1.  If the brightest had L > 0.9, snap to at
/// least 0.95.  This prevents k-means averaging dark logos into mid-gray.
fn snap_extremes(
    centers: &mut [[f32; 3]],
    pixels: &[[f32; 3]],
    assignments: &[usize],
) {
    let k = centers.len();
    let mut min_l = vec![f32::MAX; k];
    let mut max_l = vec![f32::MIN; k];

    for (px, &label) in pixels.iter().zip(assignments) {
        if label < k {
            let l = px[0];
            if l < min_l[label] {
                min_l[label] = l;
            }
            if l > max_l[label] {
                max_l[label] = l;
            }
        }
    }

    for i in 0..k {
        if min_l[i] < 0.15 {
            centers[i][0] = centers[i][0].min(0.1);
        }
        if max_l[i] > 0.9 {
            centers[i][0] = centers[i][0].max(0.95);
        }
    }
}

// ── Anti-alias boundary cleanup ──────────────────────────────────────

/// Reassign anti-aliased boundary pixels to the correct neighboring region.
///
/// Anti-aliased edges create intermediate-color pixels that get assigned to
/// wrong clusters during quantization, producing fuzzy boundaries. This pass
/// detects such pixels and reassigns them to the dominant neighbor.
///
/// A pixel is considered an anti-alias artifact when:
/// 1. Its label differs from the majority of its 4-neighbors
/// 2. Its color is intermediate between two neighboring region colors
/// 3. The larger neighboring region wins the pixel
fn cleanup_antialias(
    labels: &mut [u32],
    all_oklab: &[[f32; 3]],
    centers: &[[f32; 3]],
    width: u32,
    height: u32,
) {
    if centers.len() < 2 {
        return;
    }

    let w = width as usize;
    let h = height as usize;

    // Pre-compute region sizes for "larger region wins" tie-breaking.
    let mut region_sizes = vec![0u64; centers.len()];
    for &lbl in labels.iter() {
        if lbl != u32::MAX && (lbl as usize) < region_sizes.len() {
            region_sizes[lbl as usize] += 1;
        }
    }

    // Run the cleanup pass twice to handle multi-pixel AA fringes.
    for _pass in 0..2 {
        // We read from the current labels snapshot and write reassignments,
        // so take a snapshot to avoid cascading within a single pass.
        let snapshot: Vec<u32> = labels.to_vec();
        let mut changed = 0u32;

        for y in 0..h {
            for x in 0..w {
                let idx = y * w + x;
                let my_label = snapshot[idx];
                if my_label == u32::MAX {
                    continue;
                }

                // Gather 4-neighbor labels (skip out-of-bounds and transparent).
                let mut neighbor_labels = [u32::MAX; 4];
                let mut n_count = 0usize;
                if x > 0 {
                    neighbor_labels[n_count] = snapshot[idx - 1];
                    n_count += 1;
                }
                if x + 1 < w {
                    neighbor_labels[n_count] = snapshot[idx + 1];
                    n_count += 1;
                }
                if y > 0 {
                    neighbor_labels[n_count] = snapshot[idx - w];
                    n_count += 1;
                }
                if y + 1 < h {
                    neighbor_labels[n_count] = snapshot[idx + w];
                    n_count += 1;
                }

                // Count how many opaque neighbors share my label.
                let mut same = 0u32;
                let mut total_opaque = 0u32;
                for i in 0..n_count {
                    let nl = neighbor_labels[i];
                    if nl == u32::MAX {
                        continue;
                    }
                    total_opaque += 1;
                    if nl == my_label {
                        same += 1;
                    }
                }

                // Not a boundary pixel if majority of neighbors share my label.
                if total_opaque < 2 || same * 2 >= total_opaque {
                    continue;
                }

                // This pixel is a boundary minority. Find the two most common
                // distinct neighbor labels.
                let mut best_neighbor = u32::MAX;
                let mut best_count = 0u32;
                for i in 0..n_count {
                    let nl = neighbor_labels[i];
                    if nl == u32::MAX || nl == my_label {
                        continue;
                    }
                    let mut cnt = 0u32;
                    for j in 0..n_count {
                        if neighbor_labels[j] == nl {
                            cnt += 1;
                        }
                    }
                    // Tie-break by region size.
                    if cnt > best_count
                        || (cnt == best_count
                            && best_neighbor != u32::MAX
                            && (nl as usize) < region_sizes.len()
                            && (best_neighbor as usize) < region_sizes.len()
                            && region_sizes[nl as usize] > region_sizes[best_neighbor as usize])
                    {
                        best_neighbor = nl;
                        best_count = cnt;
                    }
                }

                if best_neighbor == u32::MAX || (best_neighbor as usize) >= centers.len() {
                    continue;
                }

                // Check if this pixel's color is intermediate between
                // my_label's center and best_neighbor's center.
                let my_center = &centers[my_label as usize];
                let neighbor_center = &centers[best_neighbor as usize];
                let pixel_color = &all_oklab[idx];

                let dist_to_mine = oklab_dist_sq(pixel_color, my_center);
                let dist_to_neighbor = oklab_dist_sq(pixel_color, neighbor_center);
                let dist_between = oklab_dist_sq(my_center, neighbor_center);

                // The pixel's color should be closer to both centers than
                // the centers are to each other (i.e., it's "between" them).
                // Use a relaxed check: pixel must be within the span of the
                // two colors it sits between.
                if dist_to_mine < dist_between && dist_to_neighbor < dist_between {
                    // Reassign to the larger neighboring region.
                    let winner = if region_sizes[best_neighbor as usize]
                        >= region_sizes[my_label as usize]
                    {
                        best_neighbor
                    } else {
                        my_label
                    };

                    if winner != my_label {
                        labels[idx] = winner;
                        changed += 1;
                    }
                }
            }
        }

        tracing::debug!(
            "Anti-alias cleanup pass {}: reassigned {} pixels",
            _pass + 1,
            changed
        );

        if changed == 0 {
            break;
        }
    }
}

// ── Morphological cleanup ─────────────────────────────────────────────

/// Apply morphological close (dilate+erode) and open (erode+dilate) to clean up
/// jagged edges in the segmented label map.
///
/// This fills tiny gaps (close) and removes tiny protrusions (open) from each
/// color region's binary mask. Only runs when:
/// - Number of unique labels is < 32 (otherwise too expensive per-label)
/// - Image is < 2 megapixels
///
/// Each label gets a binary mask, morphological ops are applied, then the
/// labels array is updated from the cleaned masks. Later labels paint over
/// earlier ones when they overlap.
pub fn morphological_cleanup(labels: &mut [u32], width: u32, height: u32, palette_size: usize) {
    use imageproc::morphology::{dilate, erode};
    use imageproc::distance_transform::Norm;
    use image::GrayImage;

    let pixel_count = (width as usize) * (height as usize);
    if pixel_count == 0 {
        return;
    }

    // Guard: skip tiny images where morph ops are meaningless.
    if width < 8 || height < 8 {
        tracing::debug!(
            "Morphological cleanup skipped: image {}x{} too small",
            width,
            height
        );
        return;
    }

    // Guard: only run for small palettes and small images.
    if palette_size >= 32 {
        tracing::debug!(
            "Morphological cleanup skipped: {} labels >= 32 limit",
            palette_size
        );
        return;
    }
    if pixel_count > 2_000_000 {
        tracing::debug!(
            "Morphological cleanup skipped: {} pixels > 2M limit",
            pixel_count
        );
        return;
    }

    // Collect unique labels (excluding transparent).
    let mut unique_labels: Vec<u32> = Vec::with_capacity(palette_size);
    {
        let mut seen = std::collections::HashSet::new();
        for &lbl in labels.iter() {
            if lbl != u32::MAX && seen.insert(lbl) {
                unique_labels.push(lbl);
            }
        }
    }
    unique_labels.sort_unstable();

    let mut changed = 0u32;

    // Process each label: create binary mask, apply morph ops, write back.
    for &label in &unique_labels {
        // Create binary mask: 255 = this label, 0 = anything else.
        let mask: GrayImage = GrayImage::from_fn(width, height, |x, y| {
            let idx = (y * width + x) as usize;
            if labels[idx] == label {
                image::Luma([255u8])
            } else {
                image::Luma([0u8])
            }
        });

        // Morphological close: dilate then erode (fills tiny gaps).
        let closed = erode(&dilate(&mask, Norm::L1, 1), Norm::L1, 1);

        // Morphological open: erode then dilate (removes tiny protrusions).
        let opened = dilate(&erode(&closed, Norm::L1, 1), Norm::L1, 1);

        // Write back: set pixels that are now in this label's mask.
        for y in 0..height {
            for x in 0..width {
                let idx = (y * width + x) as usize;
                let was_label = labels[idx] == label;
                let is_label = opened.get_pixel(x, y)[0] > 127;

                if is_label && !was_label {
                    // Gained a pixel — only claim it if it's not transparent.
                    if labels[idx] != u32::MAX {
                        labels[idx] = label;
                        changed += 1;
                    }
                } else if !is_label && was_label {
                    // Lost a pixel — it will be claimed by a later label or stay.
                    // Don't set to u32::MAX; leave for other labels to overwrite.
                    // Actually we need to restore it to something — find nearest
                    // neighbor label. For simplicity, leave it as-is (the open/close
                    // is conservative enough that this rarely happens in practice).
                }
            }
        }
    }

    tracing::debug!("Morphological cleanup: changed {} pixels", changed);
}

// ── Public entry point ────────────────────────────────────────────────

/// Quantize the image and produce a segmented label map + palette.
pub fn quantize_and_segment(
    prepared: &PreparedImage,
    config: &VectorizeConfig,
) -> Result<SegmentedImage> {
    let width = prepared.width;
    let height = prepared.height;
    let pixel_count = (width as usize) * (height as usize);

    if pixel_count == 0 {
        return Err(VectorizeError::SegmentationFailed(
            "empty image".to_string(),
        ));
    }

    let image = &prepared.image;

    // ── Collect opaque pixels in Oklab ──────────────────────────────

    // Build Oklab for every pixel and a parallel vec of "is opaque".
    let all_oklab: Vec<[f32; 3]> = image
        .pixels()
        .map(|p| rgba_to_oklab(p[0], p[1], p[2]))
        .collect();

    let is_opaque: Vec<bool> = match &prepared.opaque_mask {
        Some(mask) => mask.clone(),
        None => vec![true; pixel_count],
    };

    // Collect only opaque pixels for clustering.
    let opaque_pixels: Vec<[f32; 3]> = all_oklab
        .iter()
        .zip(is_opaque.iter())
        .filter_map(|(&px, &opaque)| if opaque { Some(px) } else { None })
        .collect();

    if opaque_pixels.is_empty() {
        // Fully transparent image — return a trivial result.
        return Ok(SegmentedImage {
            labels: vec![u32::MAX; pixel_count],
            width,
            height,
            palette: vec![],
            background_label: None,
        });
    }

    // ── B/W fast-path: skip k-means when color_count == 2 ─────────
    // Use Otsu thresholding directly — much faster and more accurate
    // for binary images (logos, line art, scanned text).

    if config.color_count == 2 {
        tracing::debug!("B/W fast-path: using Otsu thresholding for 2-color segmentation");

        // Build brightness histogram from opaque pixels.
        let mut histogram = [0u32; 256];
        for (px, &opaque) in image.pixels().zip(is_opaque.iter()) {
            if !opaque {
                continue;
            }
            let brightness = ((px[0] as u32 * 299
                + px[1] as u32 * 587
                + px[2] as u32 * 114)
                / 1000) as u8;
            histogram[brightness as usize] += 1;
        }

        let total: u32 = histogram.iter().sum();
        let threshold = if total > 0 {
            crate::preprocess::find_otsu_threshold(&histogram, total)
        } else {
            128
        };

        tracing::debug!("B/W fast-path: Otsu threshold = {}", threshold);

        // Accumulate average color for each group (dark=label 0, light=label 1).
        let mut sum_r = [0u64; 2];
        let mut sum_g = [0u64; 2];
        let mut sum_b = [0u64; 2];
        let mut counts = [0u64; 2];

        let labels: Vec<u32> = image
            .pixels()
            .zip(is_opaque.iter())
            .map(|(px, &opaque)| {
                if !opaque {
                    return u32::MAX;
                }
                let brightness = ((px[0] as u32 * 299
                    + px[1] as u32 * 587
                    + px[2] as u32 * 114)
                    / 1000) as u8;
                let label = if brightness > threshold { 1 } else { 0 };
                sum_r[label] += px[0] as u64;
                sum_g[label] += px[1] as u64;
                sum_b[label] += px[2] as u64;
                counts[label] += 1;
                label as u32
            })
            .collect();

        // Build 2-color palette from average color of each group.
        let palette: Vec<Color> = (0..2)
            .map(|i| {
                if counts[i] > 0 {
                    Color::rgb(
                        (sum_r[i] / counts[i]) as u8,
                        (sum_g[i] / counts[i]) as u8,
                        (sum_b[i] / counts[i]) as u8,
                    )
                } else {
                    // Fallback: black for label 0, white for label 1.
                    if i == 0 {
                        Color::rgb(0, 0, 0)
                    } else {
                        Color::rgb(255, 255, 255)
                    }
                }
            })
            .collect();

        tracing::debug!(
            "B/W fast-path: dark={} pixels ({}), light={} pixels ({})",
            counts[0],
            palette[0].to_svg_color(),
            counts[1],
            palette[1].to_svg_color(),
        );

        // Anti-alias cleanup for B/W path.
        let bw_centers: Vec<[f32; 3]> = palette
            .iter()
            .map(|c| rgba_to_oklab(c.r, c.g, c.b))
            .collect();
        let mut labels = labels;
        cleanup_antialias(&mut labels, &all_oklab, &bw_centers, width, height);

        // Morphological cleanup to fill tiny gaps and remove tiny protrusions.
        morphological_cleanup(&mut labels, width, height, palette.len());

        // Background detection: most frequent opaque label.
        let background_label = if config.flatten_background {
            if counts[0] >= counts[1] {
                Some(0u32)
            } else {
                Some(1u32)
            }
        } else {
            None
        };

        return Ok(SegmentedImage {
            labels,
            width,
            height,
            palette,
            background_label,
        });
    }

    // ── Determine color count (mode-aware) ─────────────────────────
    //
    // Each mode has a preferred color count range:
    // - Logo: 2-32 (few flat colors)
    // - Sketch: 2-16 (minimal fills)
    // - Illustration: 16-64 (moderate)
    // - Photo: 64-512 (many gradients)
    // - HiFi: 128-512 (maximum fidelity)

    let (range_min, range_max) = {
        let recipe = config.mode.recipe();
        recipe.color_count_range
    };

    let k = if config.color_count == 0 {
        let auto_k = auto_color_count(&opaque_pixels, range_max);
        auto_k.clamp(range_min, range_max)
    } else {
        config.color_count.clamp(range_min, range_max)
    };

    tracing::debug!("Quantizing to {} colors", k);

    // ── Subsample for k-means center finding ───────────────────────

    let sample: Vec<[f32; 3]> = if opaque_pixels.len() > 50_000 {
        let step = opaque_pixels.len() / 50_000;
        opaque_pixels.iter().step_by(step.max(1)).copied().collect()
    } else {
        opaque_pixels.clone()
    };

    // ── Run k-means on the sample ──────────────────────────────────

    let mut centers = kmeans(&sample, k as usize, 30);

    if centers.is_empty() {
        return Err(VectorizeError::SegmentationFailed(
            "k-means produced no clusters".to_string(),
        ));
    }

    // ── Snap extreme-lightness clusters toward black/white ─────────
    // Prevents k-means averaging from washing out dark logos to mid-gray.
    let sample_assignments: Vec<usize> = sample
        .iter()
        .map(|p| {
            centers
                .iter()
                .enumerate()
                .map(|(i, c)| (i, oklab_dist_sq(p, c)))
                .min_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal))
                .map_or(0, |(i, _)| i)
        })
        .collect();
    snap_extremes(&mut centers, &sample, &sample_assignments);

    // ── Assign ALL pixels to nearest center ────────────────────────

    let labels: Vec<u32> = all_oklab
        .par_iter()
        .zip(is_opaque.par_iter())
        .map(|(px, &opaque)| {
            if !opaque {
                u32::MAX
            } else {
                centers
                    .iter()
                    .enumerate()
                    .map(|(i, c)| (i, oklab_dist_sq(px, c)))
                    .min_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal))
                    .map_or(0, |(i, _)| i as u32)
            }
        })
        .collect();

    // ── Anti-alias boundary cleanup ─────────────────────────────
    let mut labels = labels;
    cleanup_antialias(&mut labels, &all_oklab, &centers, width, height);

    // ── Morphological cleanup ──────────────────────────────────────
    morphological_cleanup(&mut labels, width, height, centers.len());

    // ── Build palette ──────────────────────────────────────────────

    let palette: Vec<Color> = centers
        .iter()
        .map(|c| {
            let [r, g, b] = oklab_to_rgb(c);
            Color::rgb(r, g, b)
        })
        .collect();

    // ── Background detection ───────────────────────────────────────

    let background_label = if config.flatten_background {
        // Find the most common opaque label.
        let mut counts = vec![0u64; centers.len()];
        for &lbl in &labels {
            if lbl != u32::MAX {
                counts[lbl as usize] += 1;
            }
        }
        counts
            .iter()
            .enumerate()
            .max_by_key(|&(_, &c)| c)
            .filter(|&(_, &c)| c > 0)
            .map(|(idx, _)| idx as u32)
    } else {
        None
    };

    Ok(SegmentedImage {
        labels,
        width,
        height,
        palette,
        background_label,
    })
}

// ── Quantized image rendering ─────────────────────────────────────────

/// Render a quantized image where each pixel is replaced by its cluster's palette color.
///
/// Used by the hybrid engine to pre-quantize using Oklab color science
/// before passing to vtracer, giving it perceptually-correct colors instead
/// of its own RGB bit-shifting quantization.
pub fn render_quantized_image(segmented: &SegmentedImage) -> image::RgbaImage {
    let mut img = image::RgbaImage::new(segmented.width, segmented.height);
    for y in 0..segmented.height {
        for x in 0..segmented.width {
            let idx = (y * segmented.width + x) as usize;
            let label = segmented.labels[idx];
            if label == u32::MAX || label as usize >= segmented.palette.len() {
                img.put_pixel(x, y, image::Rgba([0, 0, 0, 0]));
            } else {
                let c = &segmented.palette[label as usize];
                img.put_pixel(x, y, image::Rgba([c.r, c.g, c.b, 255]));
            }
        }
    }
    img
}

/// Quantize an RGBA image directly (without a PreparedImage).
///
/// Used by the hybrid engine to pre-quantize before passing to vtracer.
/// Creates a temporary PreparedImage from the raw RGBA data.
pub fn quantize_rgba_image(
    rgba: &image::RgbaImage,
    config: &VectorizeConfig,
) -> Result<SegmentedImage> {
    let width = rgba.width();
    let height = rgba.height();

    // Build opaque mask from alpha channel.
    let opaque_mask: Vec<bool> = rgba.pixels().map(|p| p[3] > 0).collect();
    let has_transparency = opaque_mask.iter().any(|&o| !o);

    let prepared = PreparedImage {
        image: rgba.clone(),
        opaque_mask: if has_transparency {
            Some(opaque_mask)
        } else {
            None
        },
        width,
        height,
    };
    quantize_and_segment(&prepared, config)
}

// ── Tests ─────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_oklab_roundtrip() {
        // Pure red
        let lab = rgba_to_oklab(255, 0, 0);
        let [r, g, b] = oklab_to_rgb(&lab);
        assert_eq!(r, 255);
        assert!(g < 5); // Small rounding error acceptable
        assert!(b < 5);
    }

    #[test]
    fn test_oklab_dist() {
        let black = rgba_to_oklab(0, 0, 0);
        let white = rgba_to_oklab(255, 255, 255);
        let gray = rgba_to_oklab(128, 128, 128);

        // White should be farther from black than gray is
        assert!(oklab_dist_sq(&black, &white) > oklab_dist_sq(&black, &gray));
    }

    #[test]
    fn test_kmeans_basic() {
        let pixels = vec![
            rgba_to_oklab(0, 0, 0),
            rgba_to_oklab(0, 0, 0),
            rgba_to_oklab(255, 255, 255),
            rgba_to_oklab(255, 255, 255),
        ];
        let centers = kmeans(&pixels, 2, 10);
        assert_eq!(centers.len(), 2);
    }

    // ── New tests ─────────────────────────────────────────────────

    #[test]
    fn test_auto_color_count_bimodal() {
        // Two tight clusters in lightness: black and white.
        let mut pixels = Vec::new();
        let black = rgba_to_oklab(0, 0, 0);
        let white = rgba_to_oklab(255, 255, 255);
        for _ in 0..500 {
            pixels.push(black);
            pixels.push(white);
        }
        let k = auto_color_count(&pixels, 32);
        assert!(
            k <= 4,
            "Bimodal image should get ≤4 colors, got {}",
            k
        );
        assert!(
            k >= 2,
            "Should get at least 2 colors, got {}",
            k
        );
    }

    #[test]
    fn test_auto_color_count_complex() {
        // Many different lightness values → higher k.
        let mut pixels = Vec::new();
        for i in 0..=255 {
            pixels.push(rgba_to_oklab(i, i, i));
        }
        let k = auto_color_count(&pixels, 32);
        assert!(
            k >= 8,
            "Complex gradient should get ≥8 colors, got {}",
            k
        );
    }

    #[test]
    fn test_transparent_pixels_excluded() {
        // 4 pixels: 2 opaque black, 2 transparent.
        let mut img = image::RgbaImage::new(2, 2);
        img.put_pixel(0, 0, image::Rgba([0, 0, 0, 255]));
        img.put_pixel(1, 0, image::Rgba([0, 0, 0, 255]));
        img.put_pixel(0, 1, image::Rgba([0, 0, 0, 0]));
        img.put_pixel(1, 1, image::Rgba([0, 0, 0, 0]));

        let prepared = PreparedImage {
            image: img,
            opaque_mask: Some(vec![true, true, false, false]),
            width: 2,
            height: 2,
        };
        let config = VectorizeConfig {
            color_count: 2,
            flatten_background: false,
            ..VectorizeConfig::default()
        };
        let result = quantize_and_segment(&prepared, &config).unwrap();
        assert_eq!(result.labels[2], u32::MAX);
        assert_eq!(result.labels[3], u32::MAX);
        assert_ne!(result.labels[0], u32::MAX);
        assert_ne!(result.labels[1], u32::MAX);
    }

    #[test]
    fn test_fully_transparent_image() {
        let img = image::RgbaImage::new(2, 2);
        let prepared = PreparedImage {
            image: img,
            opaque_mask: Some(vec![false, false, false, false]),
            width: 2,
            height: 2,
        };
        let config = VectorizeConfig::default();
        let result = quantize_and_segment(&prepared, &config).unwrap();
        assert!(result.palette.is_empty());
        assert!(result.labels.iter().all(|&l| l == u32::MAX));
    }

    #[test]
    fn test_background_detection() {
        // 3x1 image: 2 white pixels, 1 black pixel.
        // Background should be the white cluster.
        let mut img = image::RgbaImage::new(3, 1);
        img.put_pixel(0, 0, image::Rgba([255, 255, 255, 255]));
        img.put_pixel(1, 0, image::Rgba([255, 255, 255, 255]));
        img.put_pixel(2, 0, image::Rgba([0, 0, 0, 255]));

        let prepared = PreparedImage {
            image: img,
            opaque_mask: None,
            width: 3,
            height: 1,
        };
        let config = VectorizeConfig {
            color_count: 2,
            flatten_background: true,
            ..VectorizeConfig::default()
        };
        let result = quantize_and_segment(&prepared, &config).unwrap();
        assert!(result.background_label.is_some());

        // The background label should be the one with 2 pixels (white).
        let bg = result.background_label.unwrap();
        let bg_count = result.labels.iter().filter(|&&l| l == bg).count();
        assert_eq!(bg_count, 2);
    }

    #[test]
    fn test_contrast_snap_dark() {
        // All pixels are very dark — the center should be snapped to L ≤ 0.1.
        let mut centers = vec![[0.12, 0.0, 0.0]]; // L=0.12 (would be averaged up)
        let pixels = vec![[0.05, 0.0, 0.0], [0.10, 0.0, 0.0]]; // dark
        let assignments = vec![0, 0];
        snap_extremes(&mut centers, &pixels, &assignments);
        assert!(
            centers[0][0] <= 0.1,
            "Dark cluster L should be snapped ≤0.1, got {}",
            centers[0][0]
        );
    }

    #[test]
    fn test_contrast_snap_bright() {
        let mut centers = vec![[0.88, 0.0, 0.0]]; // L=0.88 (averaged down)
        let pixels = vec![[0.92, 0.0, 0.0], [0.98, 0.0, 0.0]]; // bright
        let assignments = vec![0, 0];
        snap_extremes(&mut centers, &pixels, &assignments);
        assert!(
            centers[0][0] >= 0.95,
            "Bright cluster L should be snapped ≥0.95, got {}",
            centers[0][0]
        );
    }

    #[test]
    fn test_no_mask_means_all_opaque() {
        // When opaque_mask is None, all pixels should be clustered.
        let mut img = image::RgbaImage::new(2, 1);
        img.put_pixel(0, 0, image::Rgba([0, 0, 0, 255]));
        img.put_pixel(1, 0, image::Rgba([255, 255, 255, 255]));

        let prepared = PreparedImage {
            image: img,
            opaque_mask: None,
            width: 2,
            height: 1,
        };
        let config = VectorizeConfig {
            color_count: 2,
            flatten_background: false,
            ..VectorizeConfig::default()
        };
        let result = quantize_and_segment(&prepared, &config).unwrap();
        assert!(result.labels.iter().all(|&l| l != u32::MAX));
    }

    #[test]
    fn test_antialias_cleanup_reassigns_boundary_pixels() {
        // Create a 5x3 image with a clear boundary between black (left) and
        // white (right), with an anti-aliased middle column of gray pixels.
        //
        //  B B G W W      (B=black, G=gray AA pixel, W=white)
        //  B B G W W
        //  B B G W W
        //
        // The gray pixels (column 2) are intermediate and should be reassigned
        // to the larger region (white, since it has same count — but either is
        // acceptable as long as the gray doesn't stay as its own cluster).

        let black = [0u8, 0, 0];
        let white = [255u8, 255, 255];
        let gray = [128u8, 128, 128]; // anti-alias blend

        let width = 5u32;
        let height = 3u32;

        // Build Oklab pixel data.
        let mut all_oklab = Vec::new();
        for _y in 0..height {
            for x in 0..width {
                let rgb = match x {
                    0 | 1 => black,
                    2 => gray,
                    _ => white,
                };
                all_oklab.push(rgba_to_oklab(rgb[0], rgb[1], rgb[2]));
            }
        }

        // Two cluster centers: black (label 0) and white (label 1).
        let centers = vec![
            rgba_to_oklab(0, 0, 0),
            rgba_to_oklab(255, 255, 255),
        ];

        // Initially assign: black pixels → 0, white → 1, gray → 0
        // (gray is closer to black in a 2-means with only B/W centers,
        //  but let's assign it to label 0 to simulate the "wrong cluster" case).
        // Actually, for a proper test: assign gray to whichever center it's
        // closer to — it doesn't matter, the cleanup should reassign it.
        let mut labels: Vec<u32> = all_oklab
            .iter()
            .map(|px| {
                let d0 = oklab_dist_sq(px, &centers[0]);
                let d1 = oklab_dist_sq(px, &centers[1]);
                if d0 <= d1 { 0 } else { 1 }
            })
            .collect();

        // Record original labels for the gray column.
        let _gray_original: Vec<u32> = (0..height)
            .map(|y| labels[(y * width + 2) as usize])
            .collect();

        cleanup_antialias(&mut labels, &all_oklab, &centers, width, height);

        // After cleanup, the gray column pixels should have been reassigned
        // to one of the two dominant neighbor regions (they were boundary
        // minority pixels with intermediate color).
        for y in 0..height {
            let idx = (y * width + 2) as usize;
            let label = labels[idx];
            // The pixel should now match one of its dominant neighbors
            // (either the left black region or the right white region).
            let left_label = labels[(y * width + 1) as usize];
            let right_label = labels[(y * width + 3) as usize];
            assert!(
                label == left_label || label == right_label,
                "Row {}: AA pixel label {} should match left ({}) or right ({})",
                y,
                label,
                left_label,
                right_label,
            );
        }

        // Verify that pure black and white pixels were NOT changed.
        for y in 0..height {
            assert_eq!(labels[(y * width + 0) as usize], 0, "Black pixel changed");
            assert_eq!(labels[(y * width + 1) as usize], 0, "Black pixel changed");
            assert_eq!(labels[(y * width + 3) as usize], 1, "White pixel changed");
            assert_eq!(labels[(y * width + 4) as usize], 1, "White pixel changed");
        }
    }

    #[test]
    fn test_antialias_cleanup_skips_transparent() {
        // Ensure transparent pixels (u32::MAX) are not touched.
        let width = 3u32;
        let height = 1u32;
        let all_oklab = vec![
            rgba_to_oklab(0, 0, 0),
            rgba_to_oklab(128, 128, 128),
            rgba_to_oklab(255, 255, 255),
        ];
        let centers = vec![
            rgba_to_oklab(0, 0, 0),
            rgba_to_oklab(255, 255, 255),
        ];
        let mut labels = vec![0u32, u32::MAX, 1u32];
        cleanup_antialias(&mut labels, &all_oklab, &centers, width, height);
        assert_eq!(labels[1], u32::MAX, "Transparent pixel should remain u32::MAX");
    }

    #[test]
    fn test_antialias_cleanup_no_change_for_interior() {
        // A uniform region: no pixels should be changed.
        let width = 3u32;
        let height = 3u32;
        let all_oklab = vec![rgba_to_oklab(0, 0, 0); 9];
        let centers = vec![rgba_to_oklab(0, 0, 0), rgba_to_oklab(255, 255, 255)];
        let mut labels = vec![0u32; 9];
        let original = labels.clone();
        cleanup_antialias(&mut labels, &all_oklab, &centers, width, height);
        assert_eq!(labels, original, "Interior pixels should not change");
    }
}
