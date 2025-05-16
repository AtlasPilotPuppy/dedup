use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::Path;
use std::process::Command;
use std::sync::Arc;

use exif::{In, Reader as ExifReader, Tag};
use hex;
use image::{self, GenericImageView};
use img_hash::{HasherConfig, ImageHash};
use infer;
use log;
use mime_guess::MimeGuess;

use crate::audio_fingerprint;
use crate::file_utils::{DuplicateSet, FileInfo};
use crate::video_fingerprint;

/// Check if ffmpeg is installed and available
pub fn is_ffmpeg_available() -> bool {
    match Command::new("ffmpeg").arg("-version").output() {
        Ok(_) => true,
        Err(_) => false,
    }
}

/// Different supported media types for deduplication
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum MediaKind {
    Image,
    Video,
    Audio,
    Unknown,
}

/// Media options for how to handle resolution preferences
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ResolutionPreference {
    Highest,
    Lowest,
    ClosestTo(u32, u32), // Width, Height
}

impl std::fmt::Display for ResolutionPreference {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Highest => write!(f, "highest"),
            Self::Lowest => write!(f, "lowest"),
            Self::ClosestTo(w, h) => write!(f, "closest to {}x{}", w, h),
        }
    }
}

/// Media options for format preferences
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FormatPreference {
    pub formats: Vec<String>, // Ordered by preference (highest first)
}

impl Default for FormatPreference {
    fn default() -> Self {
        Self {
            formats: vec![
                // Raw formats
                "raw".to_string(),
                "arw".to_string(),
                "cr2".to_string(),
                "nef".to_string(),
                "orf".to_string(),
                "rw2".to_string(),
                // Lossless formats
                "png".to_string(),
                "tiff".to_string(),
                "bmp".to_string(),
                // Common formats
                "jpg".to_string(),
                "jpeg".to_string(),
                "mp4".to_string(),
                "mov".to_string(),
                "mp3".to_string(),
                "flac".to_string(),
                "wav".to_string(),
            ],
        }
    }
}

/// Media deduplication settings
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MediaDedupOptions {
    pub enabled: bool,
    pub resolution_preference: ResolutionPreference,
    pub format_preference: FormatPreference,
    pub similarity_threshold: u32, // 0-100, where 100 is exact match
}

impl Default for MediaDedupOptions {
    fn default() -> Self {
        Self {
            enabled: false,
            resolution_preference: ResolutionPreference::Highest,
            format_preference: FormatPreference::default(),
            similarity_threshold: 90, // Default to 90% similarity
        }
    }
}

/// Media file metadata
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MediaMetadata {
    pub kind: MediaKind,
    pub width: Option<u32>,
    pub height: Option<u32>,
    pub format: String,
    pub duration: Option<f64>, // For video/audio
    pub bitrate: Option<u32>,
    pub perceptual_hash: Option<String>,
    pub fingerprint: Option<Vec<u8>>,
}

/// Extended file info with media metadata
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MediaFileInfo {
    pub file_info: FileInfo,
    pub metadata: Option<MediaMetadata>,
}

impl From<FileInfo> for MediaFileInfo {
    fn from(file_info: FileInfo) -> Self {
        Self {
            file_info,
            metadata: None,
        }
    }
}

/// Get media type from file extension and content analysis
pub fn detect_media_type(path: &Path) -> MediaKind {
    // First try with infer (content-based detection)
    if let Ok(content) = std::fs::read(path) {
        match infer::get(&content) {
            Some(info) => match info.mime_type() {
                m if m.starts_with("image/") => return MediaKind::Image,
                m if m.starts_with("video/") => return MediaKind::Video,
                m if m.starts_with("audio/") => return MediaKind::Audio,
                _ => {}
            },
            None => {}
        }
    }

    // Fall back to extension-based detection if content analysis failed
    if let Some(extension) = path.extension() {
        if let Some(ext_str) = extension.to_str() {
            let mime = MimeGuess::from_ext(ext_str).first_or_octet_stream();
            let type_str = mime.type_().as_str();

            if type_str.starts_with("image") {
                return MediaKind::Image;
            } else if type_str.starts_with("video") {
                return MediaKind::Video;
            } else if type_str.starts_with("audio") {
                return MediaKind::Audio;
            }
        }
    }

    MediaKind::Unknown
}

/// Extract image dimensions and other metadata
pub fn extract_image_metadata(path: &Path) -> Result<MediaMetadata> {
    let format = path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("unknown")
        .to_lowercase();

    // Try to open the image
    let img = image::open(path).with_context(|| format!("Failed to open image: {:?}", path))?;

    let (width, height) = img.dimensions();

    // Calculate perceptual hash
    let hasher = HasherConfig::new().to_hasher();

    // Convert image to img_hash-compatible format
    // Create an img_hash::image::DynamicImage directly using the raw image
    let img_hash_img = {
        let rgba8 = img.to_rgba8();
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
    let hash_str = hex::encode(hash.as_bytes());

    // Try to extract EXIF data (not crucial, continue if it fails)
    let _bitrate: Option<u32> = None;
    if let Ok(file) = std::fs::File::open(path) {
        if let Ok(exif) = ExifReader::new().read_from_container(&mut std::io::BufReader::new(&file))
        {
            // Example of extracting some EXIF data if available
            if let Some(field) = exif.get_field(Tag::XResolution, In::PRIMARY) {
                // Check if the field has a rational value
                if let Some(width) = field.value.get_uint(0) {
                    // Could extract resolution or other metadata if needed
                    // Not used here but showing how to access EXIF data
                    log::debug!("Image resolution: {}", width);
                }
            }
        }
    }

    Ok(MediaMetadata {
        kind: MediaKind::Image,
        width: Some(width),
        height: Some(height),
        format,
        duration: None, // Images don't have duration
        bitrate: None,
        perceptual_hash: Some(hash_str),
        fingerprint: None, // Not used for images
    })
}

/// Extract video metadata
pub fn extract_video_metadata(path: &Path) -> Result<MediaMetadata> {
    let format = path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("unknown")
        .to_lowercase();

    // Check if ffmpeg is available
    if !is_ffmpeg_available() {
        return Err(anyhow::anyhow!(
            "ffmpeg is required for video processing but is not available"
        ));
    }

    // Extract metadata using our video_fingerprint module
    let (width, height, duration, bitrate) = video_fingerprint::extract_video_metadata(path)?;

    // Generate fingerprint
    let fingerprint = video_fingerprint::fingerprint_video(path)?;

    Ok(MediaMetadata {
        kind: MediaKind::Video,
        width,
        height,
        format,
        duration,
        bitrate,
        perceptual_hash: None,
        fingerprint: Some(fingerprint),
    })
}

/// Extract audio metadata
pub fn extract_audio_metadata(path: &Path) -> Result<MediaMetadata> {
    let format = path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("unknown")
        .to_lowercase();

    // Check if we have ffmpeg or chromaprint available
    if !is_ffmpeg_available() {
        // Try to run fpcalc to see if chromaprint is available
        let chromaprint_available = Command::new("fpcalc").arg("-version").output().is_ok();

        if !chromaprint_available {
            return Err(anyhow::anyhow!(
                "Neither ffmpeg nor chromaprint is available for audio processing"
            ));
        }
    }

    // Use our audio_fingerprint module to create fingerprint
    let fingerprint = audio_fingerprint::fingerprint_file(path)?;

    // Extract additional metadata if ffmpeg is available
    let mut duration = None;
    let mut bitrate = None;

    if is_ffmpeg_available() {
        // Use ffprobe to get audio metadata
        let output = Command::new("ffprobe")
            .args(&[
                "-v",
                "error",
                "-select_streams",
                "a:0", // First audio stream
                "-show_entries",
                "stream=duration,bit_rate",
                "-of",
                "json",
                path.to_str().unwrap(),
            ])
            .output();

        if let Ok(output) = output {
            if output.status.success() {
                let output_str = String::from_utf8_lossy(&output.stdout);
                if let Ok(json) = serde_json::from_str::<serde_json::Value>(&output_str) {
                    // Extract duration
                    duration = json["streams"][0]["duration"]
                        .as_str()
                        .and_then(|d| d.parse::<f64>().ok())
                        .or_else(|| json["streams"][0]["duration"].as_f64());

                    // Extract bitrate
                    bitrate = json["streams"][0]["bit_rate"]
                        .as_str()
                        .and_then(|b| b.parse::<u32>().ok())
                        .or_else(|| json["streams"][0]["bit_rate"].as_u64().map(|b| b as u32));
                }
            }
        }
    }

    Ok(MediaMetadata {
        kind: MediaKind::Audio,
        width: None,  // Audio has no dimensions
        height: None, // Audio has no dimensions
        format,
        duration,
        bitrate,
        perceptual_hash: None,
        fingerprint: Some(fingerprint),
    })
}

/// Extract media metadata from file
pub fn extract_media_metadata(path: &Path) -> Result<MediaMetadata> {
    let media_kind = detect_media_type(path);

    match media_kind {
        MediaKind::Image => extract_image_metadata(path),
        MediaKind::Video => extract_video_metadata(path),
        MediaKind::Audio => extract_audio_metadata(path),
        MediaKind::Unknown => Err(anyhow::anyhow!("Unknown media type for path: {:?}", path)),
    }
}

/// Calculate similarity between two image perceptual hashes (0-100)
pub fn calculate_image_similarity(hash1: &str, hash2: &str) -> u32 {
    // Convert hex string hashes back to ImageHash
    let parse_hash = |hash_str: &str| -> Option<ImageHash> {
        let bytes = (0..hash_str.len())
            .step_by(2)
            .map(|i| u8::from_str_radix(&hash_str[i..i + 2], 16).ok())
            .collect::<Option<Vec<u8>>>()?;

        ImageHash::from_bytes(&bytes).ok()
    };

    if let (Some(img_hash1), Some(img_hash2)) = (parse_hash(hash1), parse_hash(hash2)) {
        // Calculate distance (0 = identical, higher = more different)
        let distance = img_hash1.dist(&img_hash2);

        // Convert to similarity percentage (0-100)
        let max_distance = 64; // Maximum Hamming distance for 8x8 hashes
        let similarity = ((max_distance - distance) as f64 / max_distance as f64) * 100.0;

        return similarity as u32;
    }

    0 // Return 0 similarity if hash parsing failed
}

/// Calculate similarity between two video fingerprints (0-100)
pub fn calculate_video_similarity(fp1: &[u8], fp2: &[u8]) -> u32 {
    // Use our video fingerprint comparison function
    (video_fingerprint::compare_fingerprints(fp1, fp2) * 100.0) as u32
}

/// Calculate similarity between two audio fingerprints (0-100)
pub fn calculate_audio_similarity(fp1: &[u8], fp2: &[u8]) -> u32 {
    // Use our audio fingerprint comparison function
    (audio_fingerprint::compare_fingerprints(fp1, fp2) * 100.0) as u32
}

/// Compare media files and determine similarity
pub fn compare_media_files(a: &MediaFileInfo, b: &MediaFileInfo) -> u32 {
    match (&a.metadata, &b.metadata) {
        (Some(meta_a), Some(meta_b)) => {
            // Must be the same media kind
            if meta_a.kind != meta_b.kind {
                return 0;
            }

            match meta_a.kind {
                MediaKind::Image => match (&meta_a.perceptual_hash, &meta_b.perceptual_hash) {
                    (Some(hash_a), Some(hash_b)) => calculate_image_similarity(hash_a, hash_b),
                    _ => 0,
                },
                MediaKind::Video => match (&meta_a.fingerprint, &meta_b.fingerprint) {
                    (Some(fp_a), Some(fp_b)) => calculate_video_similarity(fp_a, fp_b),
                    _ => 0,
                },
                MediaKind::Audio => match (&meta_a.fingerprint, &meta_b.fingerprint) {
                    (Some(fp_a), Some(fp_b)) => calculate_audio_similarity(fp_a, fp_b),
                    _ => 0,
                },
                MediaKind::Unknown => 0,
            }
        }
        _ => 0, // No metadata to compare
    }
}

/// Determine which file to keep among similar media files
pub fn determine_preferred_media_file<'a>(
    files: &'a [MediaFileInfo],
    options: &'a MediaDedupOptions,
) -> Option<&'a MediaFileInfo> {
    if files.is_empty() {
        return None;
    }

    // Filter files with media metadata
    let files_with_metadata: Vec<_> = files.iter().filter(|f| f.metadata.is_some()).collect();

    if files_with_metadata.is_empty() {
        return files.first(); // Fall back to first file if no metadata
    }

    // First apply format preference
    let format_ranks: HashMap<String, usize> = options
        .format_preference
        .formats
        .iter()
        .enumerate()
        .map(|(i, fmt)| (fmt.clone(), i))
        .collect();

    // Helper function to get format rank (lower is better)
    let get_format_rank = |file: &&MediaFileInfo| -> usize {
        let format = match &file.metadata {
            Some(meta) => &meta.format,
            None => return usize::MAX, // No metadata = lowest rank
        };
        *format_ranks.get(format).unwrap_or(&usize::MAX)
    };

    // Sort by format preference first
    let mut preferred_format_files = files_with_metadata.clone();
    preferred_format_files.sort_by_key(get_format_rank);

    // If we have preferred format files, filter to those with the best format
    let best_format_rank = get_format_rank(&preferred_format_files[0]);
    let best_format_files: Vec<_> = preferred_format_files
        .into_iter()
        .filter(|f| get_format_rank(f) == best_format_rank)
        .collect();

    // If we have multiple files with same format, apply resolution preference
    if best_format_files.len() > 1 {
        match options.resolution_preference {
            ResolutionPreference::Highest => {
                // Find file with highest resolution
                best_format_files
                    .into_iter()
                    .max_by_key(|file| match &file.metadata {
                        Some(meta) => meta.width.unwrap_or(0) * meta.height.unwrap_or(0),
                        None => 0,
                    })
            }
            ResolutionPreference::Lowest => {
                // Find file with lowest resolution
                best_format_files
                    .into_iter()
                    .min_by_key(|file| match &file.metadata {
                        Some(meta) => {
                            meta.width.unwrap_or(u32::MAX) * meta.height.unwrap_or(u32::MAX)
                        }
                        None => u32::MAX,
                    })
            }
            ResolutionPreference::ClosestTo(target_width, target_height) => {
                // Find file with resolution closest to target
                best_format_files.into_iter().min_by_key(|file| {
                    match &file.metadata {
                        Some(meta) => {
                            let w = meta.width.unwrap_or(0);
                            let h = meta.height.unwrap_or(0);
                            let dw = if w > target_width {
                                w - target_width
                            } else {
                                target_width - w
                            };
                            let dh = if h > target_height {
                                h - target_height
                            } else {
                                target_height - h
                            };
                            dw * dw + dh * dh // Squared distance
                        }
                        None => u32::MAX,
                    }
                })
            }
        }
    } else {
        // Only one file with best format
        best_format_files.into_iter().next()
    }
}

/// Find similar media files in a directory
pub fn find_similar_media_files(
    file_infos: &[FileInfo],
    options: &MediaDedupOptions,
    progress_callback: Option<Box<dyn Fn(usize, usize) + Send>>,
) -> Result<Vec<Vec<MediaFileInfo>>> {
    if !options.enabled {
        return Ok(Vec::new());
    }

    log::info!(
        "Starting media deduplication with threshold: {}%",
        options.similarity_threshold
    );

    // Check if ffmpeg is available if we're processing videos
    let has_video_files = file_infos.iter().any(|f| {
        let kind = detect_media_type(&f.path);
        kind == MediaKind::Video
    });

    if has_video_files && !is_ffmpeg_available() {
        log::warn!("FFmpeg is not installed. Video deduplication will be limited.");
    }

    // Convert FileInfo to MediaFileInfo and extract metadata
    let total_files = file_infos.len();
    let mut processed = 0;

    // Create a thread-safe wrapper for the progress callback
    let progress_callback = progress_callback.map(|cb| {
        let cb = Arc::new(cb);
        move |count, total| {
            let cb = cb.clone();
            cb(count, total);
        }
    });

    // Process files sequentially instead of in parallel to avoid the Sync constraint
    let media_files: Vec<MediaFileInfo> = file_infos
        .iter()
        .map(|file_info| {
            let mut media_file = MediaFileInfo::from(file_info.clone());

            // Only process media files
            let media_kind = detect_media_type(&file_info.path);
            if media_kind != MediaKind::Unknown {
                media_file.metadata = match extract_media_metadata(&file_info.path) {
                    Ok(metadata) => Some(metadata),
                    Err(e) => {
                        log::warn!(
                            "Failed to extract media metadata for {:?}: {}",
                            file_info.path,
                            e
                        );
                        None
                    }
                };
            }

            // Update progress
            processed += 1;
            if let Some(cb) = &progress_callback {
                cb(processed, total_files);
            }

            media_file
        })
        .filter(|media_file| media_file.metadata.is_some())
        .collect();

    log::info!("Extracted metadata for {} media files", media_files.len());

    // Group by media type for more efficient comparison
    let mut image_files: Vec<_> = Vec::new();
    let mut video_files: Vec<_> = Vec::new();
    let mut audio_files: Vec<_> = Vec::new();

    for file in &media_files {
        if let Some(metadata) = &file.metadata {
            match metadata.kind {
                MediaKind::Image => image_files.push(file),
                MediaKind::Video => video_files.push(file),
                MediaKind::Audio => audio_files.push(file),
                _ => {}
            }
        }
    }

    log::info!(
        "Media file count: {} images, {} videos, {} audio files",
        image_files.len(),
        video_files.len(),
        audio_files.len()
    );

    // Create similarity groups
    let mut similar_groups: Vec<Vec<MediaFileInfo>> = Vec::new();

    // Process each media type separately
    process_media_type_similarity(&image_files, &options, &mut similar_groups)?;
    process_media_type_similarity(&video_files, &options, &mut similar_groups)?;
    process_media_type_similarity(&audio_files, &options, &mut similar_groups)?;

    log::info!(
        "Found {} groups of similar media files.",
        similar_groups.len()
    );

    Ok(similar_groups)
}

/// Helper function to process similarity for a specific media type
pub fn process_media_type_similarity(
    files: &[&MediaFileInfo],
    options: &MediaDedupOptions,
    similar_groups: &mut Vec<Vec<MediaFileInfo>>,
) -> Result<()> {
    if files.len() < 2 {
        return Ok(());
    }

    // Track which files have been assigned to groups
    let mut processed = vec![false; files.len()];

    // Compare each file against others
    for i in 0..files.len() {
        if processed[i] {
            continue;
        }

        let mut current_group = Vec::new();
        current_group.push(files[i].clone());
        processed[i] = true;

        for j in i + 1..files.len() {
            if processed[j] {
                continue;
            }

            let similarity = compare_media_files(files[i], files[j]);
            if similarity >= options.similarity_threshold {
                current_group.push(files[j].clone());
                processed[j] = true;
            }
        }

        if current_group.len() > 1 {
            similar_groups.push(current_group);
        }
    }

    Ok(())
}

/// Convert media similar groups to duplicate sets
pub fn convert_to_duplicate_sets(
    similar_groups: &[Vec<MediaFileInfo>],
    options: &MediaDedupOptions,
) -> Vec<DuplicateSet> {
    let mut duplicate_sets = Vec::new();

    for group in similar_groups {
        if group.len() < 2 {
            continue;
        }

        // Determine which file to keep based on preferences
        let kept_file = determine_preferred_media_file(group, options);

        if let Some(kept) = kept_file {
            // Create a duplicate set
            let mut file_infos = group
                .iter()
                .map(|mf| mf.file_info.clone())
                .collect::<Vec<_>>();

            // Ensure the kept file is first (for UI presentation)
            if let Some(kept_idx) = file_infos
                .iter()
                .position(|f| f.path == kept.file_info.path)
            {
                let kept_file_info = file_infos.remove(kept_idx);
                file_infos.insert(0, kept_file_info);
            }

            // Create a fake "hash" for media sets based on the first file in the group
            let hash = format!(
                "media_{}",
                group[0].file_info.path.to_string_lossy().to_string()
            );
            let size = group[0].file_info.size;

            duplicate_sets.push(DuplicateSet {
                files: file_infos,
                size,
                hash,
            });
        }
    }

    duplicate_sets
}

/// Update Cli to add media deduplication options
pub fn add_media_options_to_cli(
    options: &mut MediaDedupOptions,
    enable: bool,
    resolution: &str,
    formats: &[String],
    threshold: u32,
) {
    options.enabled = enable;

    // Parse resolution preference
    match resolution {
        "highest" => options.resolution_preference = ResolutionPreference::Highest,
        "lowest" => options.resolution_preference = ResolutionPreference::Lowest,
        custom => {
            // Try to parse "WxH" format
            if let Some((width, height)) = custom.split_once('x') {
                if let (Ok(w), Ok(h)) = (width.parse::<u32>(), height.parse::<u32>()) {
                    options.resolution_preference = ResolutionPreference::ClosestTo(w, h);
                }
            }
        }
    }

    // Update format preferences if provided
    if !formats.is_empty() {
        options.format_preference.formats = formats.to_vec();
    }

    // Update similarity threshold
    if threshold > 0 && threshold <= 100 {
        options.similarity_threshold = threshold;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    use std::time::SystemTime;

    // Helper to create a test file
    fn create_test_file_info(path: &str, size: u64) -> FileInfo {
        FileInfo {
            path: PathBuf::from(path),
            size,
            hash: Some("test_hash".to_string()),
            modified_at: Some(SystemTime::now()),
            created_at: Some(SystemTime::now()),
        }
    }

    #[test]
    fn test_ffmpeg_availability() {
        // This test just checks if the function runs without crashing
        let available = is_ffmpeg_available();
        // We can't assert a specific value as it depends on the system
        println!("FFmpeg available: {}", available);
    }

    #[test]
    fn test_media_kind_from_extension() {
        // Test image detection
        assert_eq!(detect_media_type(Path::new("test.jpg")), MediaKind::Image);
        assert_eq!(detect_media_type(Path::new("test.png")), MediaKind::Image);

        // Test video detection
        assert_eq!(detect_media_type(Path::new("test.mp4")), MediaKind::Video);
        assert_eq!(detect_media_type(Path::new("test.mov")), MediaKind::Video);

        // Test audio detection
        assert_eq!(detect_media_type(Path::new("test.mp3")), MediaKind::Audio);
        assert_eq!(detect_media_type(Path::new("test.wav")), MediaKind::Audio);

        // Test unknown
        assert_eq!(detect_media_type(Path::new("test.txt")), MediaKind::Unknown);
    }

    #[test]
    fn test_format_preference() {
        // Test the default format preferences
        let format_pref = FormatPreference::default();

        // Check if raw formats are higher priority than jpg
        assert!(
            format_pref.formats.iter().position(|f| f == "raw").unwrap()
                < format_pref.formats.iter().position(|f| f == "jpg").unwrap()
        );

        // Check if png is higher priority than jpg (lossless over lossy)
        assert!(
            format_pref.formats.iter().position(|f| f == "png").unwrap()
                < format_pref.formats.iter().position(|f| f == "jpg").unwrap()
        );
    }

    #[test]
    fn test_resolution_preference_display() {
        assert_eq!(ResolutionPreference::Highest.to_string(), "highest");
        assert_eq!(ResolutionPreference::Lowest.to_string(), "lowest");
        assert_eq!(
            ResolutionPreference::ClosestTo(1280, 720).to_string(),
            "closest to 1280x720"
        );
    }

    #[test]
    fn test_media_dedup_options_default() {
        let options = MediaDedupOptions::default();
        assert_eq!(options.enabled, false);
        assert_eq!(options.similarity_threshold, 90);

        // Test that resolution preference is highest by default
        match options.resolution_preference {
            ResolutionPreference::Highest => (),
            _ => panic!("Default resolution preference should be Highest"),
        }
    }
}
