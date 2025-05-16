use std::path::Path;
use std::process::Command;
use anyhow::{Result, Context};

/// Simple audio fingerprinting module using chromaprint/fpcalc if available
/// or ffmpeg's ebur128 filter as a fallback

/// Generate an audio fingerprint from a file
pub fn fingerprint_file(path: &Path) -> Result<Vec<u8>> {
    // First try with chromaprint/fpcalc if available
    if let Ok(fp) = fingerprint_with_chromaprint(path) {
        return Ok(fp);
    }
    
    // Fall back to ffmpeg if chromaprint is not available
    if crate::media_dedup::is_ffmpeg_available() {
        fingerprint_with_ffmpeg(path)
    } else {
        Err(anyhow::anyhow!("Neither chromaprint nor ffmpeg are available for audio fingerprinting"))
    }
}

/// Generate fingerprint using chromaprint/fpcalc tool
fn fingerprint_with_chromaprint(path: &Path) -> Result<Vec<u8>> {
    // Try to execute fpcalc (part of chromaprint)
    let output = Command::new("fpcalc")
        .arg("-raw")
        .arg("-json")
        .arg(path)
        .output()
        .context("Failed to execute fpcalc. Is chromaprint installed?")?;
    
    if !output.status.success() {
        return Err(anyhow::anyhow!(
            "fpcalc failed: {}",
            String::from_utf8_lossy(&output.stderr)
        ));
    }
    
    let output_str = String::from_utf8_lossy(&output.stdout);
    
    // Parse the JSON output
    let json: serde_json::Value = serde_json::from_str(&output_str)
        .context("Failed to parse fpcalc JSON output")?;
    
    // Extract the fingerprint
    if let Some(fingerprint_str) = json["fingerprint"].as_str() {
        // Convert from base64 or hex string to bytes
        let fingerprint = hex::decode(fingerprint_str)
            .or_else(|_| base64::decode(fingerprint_str))
            .context("Failed to decode fingerprint")?;
        
        Ok(fingerprint)
    } else {
        Err(anyhow::anyhow!("No fingerprint found in fpcalc output"))
    }
}

/// Generate a simple fingerprint using ffmpeg's ebur128 filter
fn fingerprint_with_ffmpeg(path: &Path) -> Result<Vec<u8>> {
    // Use ffmpeg to analyze audio and extract loudness information
    // This is a simpler alternative to chromaprint but less accurate
    let output = Command::new("ffmpeg")
        .args(&[
            "-i", path.to_str().unwrap(),
            "-filter:a", "ebur128=metadata=1",
            "-f", "null", "-"
        ])
        .output()
        .context("Failed to execute ffmpeg")?;
    
    if !output.status.success() {
        return Err(anyhow::anyhow!(
            "ffmpeg failed: {}",
            String::from_utf8_lossy(&output.stderr)
        ));
    }
    
    // Extract loudness stats from stderr where ffmpeg writes ebur128 output
    let stderr = String::from_utf8_lossy(&output.stderr);
    
    // Create a simple fingerprint from the loudness stats
    // This is much less accurate than chromaprint but can work as a fallback
    let mut fingerprint = Vec::new();
    
    // Parse integrated loudness
    if let Some(pos) = stderr.find("I:") {
        if let Some(end) = stderr[pos..].find(" ") {
            let loudness_str = &stderr[pos+2..pos+end];
            if let Ok(loudness) = loudness_str.parse::<f32>() {
                // Add the float bytes to our fingerprint
                fingerprint.extend_from_slice(&loudness.to_le_bytes());
            }
        }
    }
    
    // Parse LRA
    if let Some(pos) = stderr.find("LRA:") {
        if let Some(end) = stderr[pos..].find(" ") {
            let lra_str = &stderr[pos+4..pos+end];
            if let Ok(lra) = lra_str.parse::<f32>() {
                fingerprint.extend_from_slice(&lra.to_le_bytes());
            }
        }
    }
    
    // If we couldn't extract anything useful, return an error
    if fingerprint.is_empty() {
        return Err(anyhow::anyhow!("Could not extract audio fingerprint from ffmpeg output"));
    }
    
    Ok(fingerprint)
}

/// Compare two audio fingerprints and return similarity (0.0-1.0)
pub fn compare_fingerprints(fp1: &[u8], fp2: &[u8]) -> f64 {
    if fp1.len() != fp2.len() || fp1.is_empty() {
        return 0.0;
    }
    
    // Count matching bytes
    let mut matches = 0;
    for (b1, b2) in fp1.iter().zip(fp2.iter()) {
        // Count how many bits match
        let xor = b1 ^ b2;
        matches += 8 - xor.count_ones();
    }
    
    // Calculate percentage of matching bits
    (matches as f64) / ((fp1.len() * 8) as f64)
}

#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_compare_fingerprints() {
        // Identical fingerprints should have 100% similarity
        let fp1 = vec![0, 1, 2, 3, 4];
        let fp2 = vec![0, 1, 2, 3, 4];
        assert_eq!(compare_fingerprints(&fp1, &fp2), 1.0);
        
        // Different fingerprints should have lower similarity
        let fp3 = vec![255, 255, 255, 255, 255];
        assert!(compare_fingerprints(&fp1, &fp3) < 0.5);
        
        // Empty fingerprints should have 0% similarity
        let empty = vec![];
        assert_eq!(compare_fingerprints(&fp1, &empty), 0.0);
    }
} 