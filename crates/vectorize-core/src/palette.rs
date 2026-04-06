//! Palette reduction: limit colors to N tones per hue group.
//!
//! Groups colors by hue, then within each hue group limits to a fixed number
//! of tonal values (highlight, midtone, shadow). This produces clean,
//! illustration-like output instead of blotchy gradients.

use crate::Color;
use std::collections::HashMap;

/// Reduce a set of colors to at most `tones_per_hue` lightness levels per hue group.
/// Returns a mapping from original color to its reduced palette color.
///
/// - `tones_per_hue`: max tones per hue bucket (e.g., 3 = highlight/mid/shadow)
/// - `hue_buckets`: number of hue sectors (12 = 30° each, 18 = 20° each)
///
/// Colors with very low saturation are grouped as "neutrals" and get their own
/// N-tone set.
pub fn reduce_palette(
    colors: &[Color],
    tones_per_hue: usize,
    hue_buckets: usize,
) -> HashMap<Color, Color> {
    if tones_per_hue == 0 || colors.is_empty() {
        return HashMap::new();
    }

    let hue_buckets = hue_buckets.max(1);

    // Convert each color to HSL and bucket by hue.
    // Bucket ID = hue_sector (0..hue_buckets) or usize::MAX for neutrals.
    let neutral_bucket = usize::MAX;
    let saturation_threshold = 0.08; // below this = neutral/gray

    struct ColorHSL {
        original: Color,
        hue: f64,
        saturation: f64,
        lightness: f64,
        bucket: usize,
    }

    let mut color_data: Vec<ColorHSL> = colors
        .iter()
        .map(|c| {
            let (h, s, l) = rgb_to_hsl(c.r, c.g, c.b);
            let bucket = if s < saturation_threshold {
                neutral_bucket
            } else {
                ((h / 360.0 * hue_buckets as f64).floor() as usize).min(hue_buckets - 1)
            };
            ColorHSL {
                original: *c,
                hue: h,
                saturation: s,
                lightness: l,
                bucket,
            }
        })
        .collect();

    // Group by bucket.
    let mut buckets: HashMap<usize, Vec<usize>> = HashMap::new();
    for (i, cd) in color_data.iter().enumerate() {
        buckets.entry(cd.bucket).or_default().push(i);
    }

    // For each bucket, quantize lightness into `tones_per_hue` bands.
    let mut remap: HashMap<Color, Color> = HashMap::new();

    for (_bucket_id, indices) in &buckets {
        if indices.is_empty() {
            continue;
        }

        // Sort by lightness.
        let mut sorted: Vec<usize> = indices.clone();
        sorted.sort_by(|&a, &b| {
            color_data[a]
                .lightness
                .partial_cmp(&color_data[b].lightness)
                .unwrap_or(std::cmp::Ordering::Equal)
        });

        if sorted.len() <= tones_per_hue {
            // Fewer colors than allowed tones — keep all as-is.
            for &idx in &sorted {
                let c = color_data[idx].original;
                remap.insert(c, c);
            }
            continue;
        }

        // Divide into N bands by lightness and pick the median color from each band.
        let band_size = sorted.len() as f64 / tones_per_hue as f64;
        let mut representatives: Vec<Color> = Vec::with_capacity(tones_per_hue);
        let mut band_ranges: Vec<(usize, usize)> = Vec::with_capacity(tones_per_hue);

        for tone in 0..tones_per_hue {
            let start = (tone as f64 * band_size).round() as usize;
            let end = (((tone + 1) as f64) * band_size).round() as usize;
            let end = end.min(sorted.len());
            band_ranges.push((start, end));

            // Pick the median color from this band.
            let mid = (start + end) / 2;
            let mid = mid.min(sorted.len() - 1);
            let median_idx = sorted[mid];

            // Average the hue/sat/lightness for the representative.
            let avg_h: f64 = sorted[start..end]
                .iter()
                .map(|&i| color_data[i].hue)
                .sum::<f64>()
                / (end - start) as f64;
            let avg_s: f64 = sorted[start..end]
                .iter()
                .map(|&i| color_data[i].saturation)
                .sum::<f64>()
                / (end - start) as f64;
            let avg_l: f64 = sorted[start..end]
                .iter()
                .map(|&i| color_data[i].lightness)
                .sum::<f64>()
                / (end - start) as f64;

            let (r, g, b) = hsl_to_rgb(avg_h, avg_s, avg_l);
            let rep = Color::rgba(r, g, b, color_data[median_idx].original.a);
            representatives.push(rep);
        }

        // Map each color in this bucket to its nearest representative.
        for (band_idx, &(start, end)) in band_ranges.iter().enumerate() {
            let rep = representatives[band_idx];
            for &sorted_idx in &sorted[start..end] {
                let orig = color_data[sorted_idx].original;
                remap.insert(orig, rep);
            }
        }
    }

    remap
}

/// Apply a palette reduction to a set of (BezPath, Color) pairs.
pub fn apply_palette_reduction(
    paths: &mut [(kurbo::BezPath, Color)],
    remap: &HashMap<Color, Color>,
) {
    for (_, color) in paths.iter_mut() {
        if let Some(new_color) = remap.get(color) {
            *color = *new_color;
        }
    }
}

// ── HSL conversion ──────────────────────────────────────────────────────────

fn rgb_to_hsl(r: u8, g: u8, b: u8) -> (f64, f64, f64) {
    let r = r as f64 / 255.0;
    let g = g as f64 / 255.0;
    let b = b as f64 / 255.0;

    let max = r.max(g).max(b);
    let min = r.min(g).min(b);
    let l = (max + min) / 2.0;

    if (max - min).abs() < 1e-10 {
        return (0.0, 0.0, l);
    }

    let d = max - min;
    let s = if l > 0.5 {
        d / (2.0 - max - min)
    } else {
        d / (max + min)
    };

    let h = if (max - r).abs() < 1e-10 {
        let mut h = (g - b) / d;
        if g < b {
            h += 6.0;
        }
        h
    } else if (max - g).abs() < 1e-10 {
        (b - r) / d + 2.0
    } else {
        (r - g) / d + 4.0
    };

    (h * 60.0, s, l)
}

fn hsl_to_rgb(h: f64, s: f64, l: f64) -> (u8, u8, u8) {
    if s.abs() < 1e-10 {
        let v = (l * 255.0).round() as u8;
        return (v, v, v);
    }

    let q = if l < 0.5 {
        l * (1.0 + s)
    } else {
        l + s - l * s
    };
    let p = 2.0 * l - q;
    let h = h / 360.0;

    let r = hue_to_rgb(p, q, h + 1.0 / 3.0);
    let g = hue_to_rgb(p, q, h);
    let b = hue_to_rgb(p, q, h - 1.0 / 3.0);

    (
        (r * 255.0).round() as u8,
        (g * 255.0).round() as u8,
        (b * 255.0).round() as u8,
    )
}

fn hue_to_rgb(p: f64, q: f64, mut t: f64) -> f64 {
    if t < 0.0 {
        t += 1.0;
    }
    if t > 1.0 {
        t -= 1.0;
    }
    if t < 1.0 / 6.0 {
        return p + (q - p) * 6.0 * t;
    }
    if t < 1.0 / 2.0 {
        return q;
    }
    if t < 2.0 / 3.0 {
        return p + (q - p) * (2.0 / 3.0 - t) * 6.0;
    }
    p
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_reduce_three_tones() {
        // 6 blues of varying lightness → should reduce to 3
        let colors = vec![
            Color::rgb(0, 0, 50),
            Color::rgb(0, 0, 80),
            Color::rgb(0, 0, 120),
            Color::rgb(0, 0, 160),
            Color::rgb(0, 0, 200),
            Color::rgb(0, 0, 240),
        ];
        let remap = reduce_palette(&colors, 3, 12);
        let unique_outputs: std::collections::HashSet<_> =
            remap.values().collect();
        assert!(unique_outputs.len() <= 3, "Expected ≤3 tones, got {}", unique_outputs.len());
    }

    #[test]
    fn test_neutrals_separate() {
        // Grays and blues should be in different buckets
        let colors = vec![
            Color::rgb(100, 100, 100), // neutral
            Color::rgb(150, 150, 150), // neutral
            Color::rgb(200, 200, 200), // neutral
            Color::rgb(0, 0, 200),     // blue
            Color::rgb(0, 0, 150),     // blue
            Color::rgb(0, 0, 100),     // blue
        ];
        let remap = reduce_palette(&colors, 2, 12);
        // Should have ≤2 neutral tones + ≤2 blue tones = ≤4 total
        let unique_outputs: std::collections::HashSet<_> =
            remap.values().collect();
        assert!(unique_outputs.len() <= 4, "Expected ≤4, got {}", unique_outputs.len());
    }

    #[test]
    fn test_hsl_roundtrip() {
        let (h, s, l) = rgb_to_hsl(100, 150, 200);
        let (r, g, b) = hsl_to_rgb(h, s, l);
        assert!((r as i32 - 100).abs() <= 1);
        assert!((g as i32 - 150).abs() <= 1);
        assert!((b as i32 - 200).abs() <= 1);
    }
}
