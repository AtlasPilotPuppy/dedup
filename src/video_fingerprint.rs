use anyhow::{Context, Result};
use std::path::Path;
use std::process::Command;

use image::DynamicImage;
use img_hash::{HashAlg, HasherConfig};

/// Video fingerprinting module using ffmpeg to extract keyframes
/// and img_hash to generate perceptual hashes for those frames

/// Extract a fingerprint from a video file
pub fn fingerprint_video(path: &Path) -> Result<Vec<u8>> {
    if !crate::media_dedup::is_ffmpeg_available() {
        return Err(anyhow::anyhow!(
            "ffmpeg is not available for video fingerprinting"
        ));
    }

    // Extract keyframes from the video using ffmpeg
    let keyframes = extract_keyframes(path)?;

    if keyframes.is_empty() {
        return Err(anyhow::anyhow!(
            "Could not extract any keyframes from video"
        ));
    }

    // Generate perceptual hash for each keyframe and combine them
    let mut fingerprint = Vec::new();
    let hasher = HasherConfig::new()
        .hash_alg(HashAlg::DoubleGradient)
        .hash_size(8, 8)
        .to_hasher();

    for frame in keyframes {
        // Convert frame to img_hash compatible format
        let img_hash_img = {
            let rgba8 = frame.to_rgba8();
            let width = rgba8.width();
            let height = rgba8.height();
            let raw_pixels = rgba8.into_raw();

            // Create a new image buffer using img_hash's image version
            let buffer = img_hash::image::ImageBuffer::from_raw(width, height, raw_pixels)
                .expect("Failed to convert image buffer");

            // Create dynamic image from buffer
            img_hash::image::DynamicImage::ImageRgba8(buffer)
        };

        // Use the compatible image format with img_hash
        let hash = hasher.hash_image(&img_hash_img);
        fingerprint.extend_from_slice(hash.as_bytes());
    }

    Ok(fingerprint)
}

/// Extract keyframes from a video using ffmpeg
fn extract_keyframes(path: &Path) -> Result<Vec<DynamicImage>> {
    // Create a temporary directory for the extracted frames
    let temp_dir = tempfile::tempdir()?;
    let output_pattern = temp_dir.path().join("keyframe%03d.png");

    // Use ffmpeg to extract I-frames (keyframes) only
    let output = Command::new("ffmpeg")
        .args([
            "-i",
            path.to_str().unwrap(),
            "-vf",
            "select=eq(pict_type\\,I)", // Only extract I-frames
            "-vsync",
            "vfr", // Variable framerate output
            "-qscale:v",
            "2", // High quality
            "-frames:v",
            "5", // Limit to 5 keyframes max
            output_pattern.to_str().unwrap(),
        ])
        .output()
        .context("Failed to execute ffmpeg for keyframe extraction")?;

    if !output.status.success() {
        return Err(anyhow::anyhow!(
            "ffmpeg keyframe extraction failed: {}",
            String::from_utf8_lossy(&output.stderr)
        ));
    }

    // Read the extracted frames
    let mut keyframes = Vec::new();
    for i in 1..=5 {
        // Try all 5 potential frames
        let frame_path = temp_dir.path().join(format!("keyframe{:03}.png", i));
        if frame_path.exists() {
            match image::open(&frame_path) {
                Ok(img) => keyframes.push(img),
                Err(e) => log::warn!("Failed to open keyframe {}: {}", i, e),
            }
        }
    }

    Ok(keyframes)
}

/// Extract video metadata using ffmpeg
pub fn extract_video_metadata(
    path: &Path,
) -> Result<(Option<u32>, Option<u32>, Option<f64>, Option<u32>)> {
    if !crate::media_dedup::is_ffmpeg_available() {
        return Err(anyhow::anyhow!(
            "ffmpeg is not available for video metadata extraction"
        ));
    }

    // Use ffmpeg to get video metadata
    let output = Command::new("ffprobe")
        .args([
            "-v",
            "error",
            "-select_streams",
            "v:0", // First video stream
            "-show_entries",
            "stream=width,height,duration,bit_rate",
            "-of",
            "json",
            path.to_str().unwrap(),
        ])
        .output()
        .context("Failed to execute ffprobe")?;

    if !output.status.success() {
        return Err(anyhow::anyhow!(
            "ffprobe failed: {}",
            String::from_utf8_lossy(&output.stderr)
        ));
    }

    let output_str = String::from_utf8_lossy(&output.stdout);
    let json: serde_json::Value =
        serde_json::from_str(&output_str).context("Failed to parse ffprobe JSON output")?;

    // Extract information
    let width = json["streams"][0]["width"].as_u64().map(|w| w as u32);
    let height = json["streams"][0]["height"].as_u64().map(|h| h as u32);

    // Duration might be stored as a string with float value
    let duration = json["streams"][0]["duration"]
        .as_str()
        .and_then(|d| d.parse::<f64>().ok())
        .or_else(|| json["streams"][0]["duration"].as_f64());

    // Bitrate might be stored as a string or number
    let bitrate = json["streams"][0]["bit_rate"]
        .as_str()
        .and_then(|b| b.parse::<u32>().ok())
        .or_else(|| json["streams"][0]["bit_rate"].as_u64().map(|b| b as u32));

    Ok((width, height, duration, bitrate))
}

/// Compare two video fingerprints
pub fn compare_fingerprints(fp1: &[u8], fp2: &[u8]) -> f64 {
    // Need at least some data to compare
    if fp1.is_empty() || fp2.is_empty() {
        return 0.0;
    }

    // Use the smaller length to avoid index out of bounds
    let min_len = std::cmp::min(fp1.len(), fp2.len());

    let mut matches = 0;
    for i in 0..min_len {
        if fp1[i] == fp2[i] {
            matches += 1;
        }
    }

    matches as f64 / min_len as f64
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_compare_fingerprints() {
        // Create similar but different fingerprints
        let fp1 = vec![0, 1, 2, 3, 4, 5, 6, 7, 8, 9];
        let fp2 = vec![0, 1, 2, 3, 5, 5, 7, 7, 8, 9]; // 8/10 match

        // Similarity should be 0.8
        let similarity = compare_fingerprints(&fp1, &fp2);
        assert_eq!(similarity, 0.8);

        // Test identical fingerprints
        let similarity = compare_fingerprints(&fp1, &fp1);
        assert_eq!(similarity, 1.0);

        // Test completely different fingerprints
        let fp3 = vec![10, 11, 12, 13, 14, 15, 16, 17, 18, 19];
        let similarity = compare_fingerprints(&fp1, &fp3);
        assert_eq!(similarity, 0.0);
    }
}
