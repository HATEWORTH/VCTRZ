//! Stage 3: Contour extraction and boundary tracing.
//!
//! For each color region in the segmented image, creates a binary mask
//! and extracts ordered boundary point sequences using imageproc's
//! contour finder (Suzuki-Abe algorithm).
//!
//! Contours are extracted as boundary point sequences for subsequent
//! curve fitting.

#![allow(clippy::cast_possible_truncation)]

use image::GrayImage;
use rayon::prelude::*;

use crate::{SegmentedImage, TracedContour, VectorizeConfig};

/// Extract contours from all color regions in the segmented image.
pub fn extract_contours(
    segmented: &SegmentedImage,
    config: &VectorizeConfig,
) -> Vec<TracedContour> {
    let width = segmented.width;
    let height = segmented.height;
    let num_colors = segmented.palette.len();

    // Determine which label to skip (background that will become a full-canvas rect)
    let skip_label: Option<u32> = if config.flatten_background {
        segmented.background_label
    } else {
        None
    };

    // Process each color label in parallel
    (0..num_colors)
        .into_par_iter()
        .flat_map(|label_idx| {
            let label = label_idx as u32;

            // Skip the background label — it will be rendered as a full-canvas rect
            if skip_label == Some(label) {
                return vec![];
            }

            let color = segmented.palette[label_idx];

            // Count pixels for area check
            let area: u32 = segmented
                .labels
                .iter()
                .filter(|&&l| l == label)
                .count() as u32;
            // Use explicit min_area if set (non-default), otherwise derive from quality.
            let min_area = if config.min_area != VectorizeConfig::default().min_area {
                config.min_area
            } else {
                config.quality.native_min_area()
            };
            if area < min_area {
                return vec![];
            }

            // Create binary mask: 255 where pixel matches this label, 0 elsewhere.
            // Transparent pixels (u32::MAX) are always background (0).
            let mask_pixels: Vec<u8> = segmented
                .labels
                .iter()
                .map(|&l| if l == label { 255u8 } else { 0u8 })
                .collect();

            let mask = GrayImage::from_raw(width, height, mask_pixels)
                .expect("mask dimensions match pixel count");

            // Extract contours using imageproc's Suzuki-Abe implementation
            let contours = imageproc::contours::find_contours::<u32>(&mask);

            contours
                .into_iter()
                .filter(|c| c.points.len() >= 3) // Need at least 3 points for a shape
                .map(|c| {
                    let is_hole =
                        c.border_type == imageproc::contours::BorderType::Hole;

                    let points: Vec<kurbo::Point> = c
                        .points
                        .iter()
                        .map(|p| kurbo::Point::new(f64::from(p.x), f64::from(p.y)))
                        .collect();

                    TracedContour {
                        points,
                        color,
                        is_hole,
                    }
                })
                .collect::<Vec<_>>()
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Color, SegmentedImage};

    #[test]
    fn test_extract_simple_rectangle() {
        // 10x10 image with a 4x4 white rectangle on black background
        let width = 10;
        let height = 10;
        let mut labels = vec![0u32; 100];
        for y in 3..7 {
            for x in 3..7 {
                labels[y * width as usize + x] = 1;
            }
        }

        let segmented = SegmentedImage {
            labels,
            width: width as u32,
            height: height as u32,
            palette: vec![Color::rgb(0, 0, 0), Color::rgb(255, 255, 255)],
            background_label: None,
        };

        let config = VectorizeConfig {
            min_area: 1,
            ..Default::default()
        };

        let contours = extract_contours(&segmented, &config);
        // Should have at least one contour for the white rectangle
        assert!(!contours.is_empty());
        // At least one contour should be white
        assert!(contours.iter().any(|c| c.color == Color::rgb(255, 255, 255)));
    }

    #[test]
    fn test_transparent_pixels_skipped() {
        // 4x4 image: label 0 and some transparent pixels (u32::MAX)
        let labels = vec![
            0, 0, u32::MAX, u32::MAX,
            0, 0, u32::MAX, u32::MAX,
            u32::MAX, u32::MAX, u32::MAX, u32::MAX,
            u32::MAX, u32::MAX, u32::MAX, u32::MAX,
        ];

        let segmented = SegmentedImage {
            labels,
            width: 4,
            height: 4,
            palette: vec![Color::rgb(255, 0, 0)],
            background_label: None,
        };

        let config = VectorizeConfig {
            min_area: 1,
            ..Default::default()
        };

        let contours = extract_contours(&segmented, &config);
        // Should find contours only for label 0, not for u32::MAX
        assert!(contours.iter().all(|c| c.color == Color::rgb(255, 0, 0)));
    }

    #[test]
    fn test_background_label_skipped() {
        let width = 10;
        let height = 10;
        let mut labels = vec![0u32; 100];
        for y in 3..7 {
            for x in 3..7 {
                labels[y * width as usize + x] = 1;
            }
        }

        let segmented = SegmentedImage {
            labels,
            width: width as u32,
            height: height as u32,
            palette: vec![Color::rgb(0, 0, 0), Color::rgb(255, 255, 255)],
            background_label: Some(0), // black is background
        };

        let config = VectorizeConfig {
            min_area: 1,
            flatten_background: true,
            ..Default::default()
        };

        let contours = extract_contours(&segmented, &config);
        // Background label 0 (black) should be skipped
        assert!(contours.iter().all(|c| c.color != Color::rgb(0, 0, 0)));
    }

}
