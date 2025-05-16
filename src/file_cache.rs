use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs::{self, File};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use crate::file_utils::FileInfo;

/// File cache entry stored for each file
#[derive(Debug, Clone, Serialize, Deserialize)]
struct FileCacheEntry {
    path: PathBuf,
    size: u64,
    hash: String,
    modified_at: Option<SystemTime>,
    algorithm: String,
}

impl FileCacheEntry {
    fn from_file_info(file_info: &FileInfo, algorithm: &str) -> Option<Self> {
        file_info.hash.as_ref().map(|hash| Self {
            path: file_info.path.clone(),
            size: file_info.size,
            hash: hash.clone(),
            modified_at: file_info.modified_at,
            algorithm: algorithm.to_string(),
        })
    }

    fn to_file_info(&self) -> FileInfo {
        FileInfo {
            path: self.path.clone(),
            size: self.size,
            hash: Some(self.hash.clone()),
            modified_at: self.modified_at,
            created_at: None, // Cache doesn't store creation time
        }
    }

    /// Check if this cache entry is still valid for the given file
    fn is_valid(&self, path: &Path, algorithm: &str) -> bool {
        if self.algorithm != algorithm {
            return false;
        }

        match fs::metadata(path) {
            Ok(metadata) => {
                // Check if file size matches
                if metadata.len() != self.size {
                    return false;
                }

                // Check if modification time matches
                if let Ok(mtime) = metadata.modified() {
                    if let Some(cached_mtime) = self.modified_at {
                        return mtime == cached_mtime;
                    }
                }

                false // No modification time to compare
            }
            Err(_) => false, // Can't read metadata, treat as invalid
        }
    }
}

/// Cache directory structure
#[derive(Debug)]
pub struct FileCache {
    cache_dir: PathBuf,
    entries: HashMap<PathBuf, FileCacheEntry>,
    algorithm: String,
    modified: bool,
}

impl FileCache {
    /// Create a new file cache using the given cache directory
    pub fn new(cache_dir: &Path, algorithm: &str) -> Result<Self> {
        // Create cache directory if it doesn't exist
        if !cache_dir.exists() {
            fs::create_dir_all(cache_dir)
                .with_context(|| format!("Failed to create cache directory: {:?}", cache_dir))?;
        }

        let cache_file = Self::cache_file_path(cache_dir, algorithm);
        let mut entries = HashMap::new();

        // Load existing cache if available
        if cache_file.exists() {
            let mut file = File::open(&cache_file)
                .with_context(|| format!("Failed to open cache file: {:?}", cache_file))?;

            let mut contents = Vec::new();
            file.read_to_end(&mut contents)
                .with_context(|| format!("Failed to read cache file: {:?}", cache_file))?;

            entries = serde_json::from_slice(&contents)
                .with_context(|| format!("Failed to parse cache file: {:?}", cache_file))
                .unwrap_or_default();

            log::info!(
                "Loaded {} entries from cache file: {:?}",
                entries.len(),
                cache_file
            );
        }

        Ok(Self {
            cache_dir: cache_dir.to_path_buf(),
            entries,
            algorithm: algorithm.to_string(),
            modified: false,
        })
    }

    /// Get the path to the cache file for a specific algorithm
    fn cache_file_path(cache_dir: &Path, algorithm: &str) -> PathBuf {
        cache_dir.join(format!("file_hashes_{}.cache", algorithm))
    }

    /// Get a file hash from the cache if available and still valid
    pub fn get_hash(&self, path: &Path) -> Option<String> {
        if let Some(entry) = self.entries.get(path) {
            if entry.is_valid(path, &self.algorithm) {
                log::debug!("Cache hit for file: {:?}", path);
                return Some(entry.hash.clone());
            } else {
                log::debug!("Cache invalid for file: {:?}", path);
            }
        }

        None
    }

    /// Get a complete FileInfo from the cache if available and still valid
    pub fn get_file_info(&self, path: &Path) -> Option<FileInfo> {
        if let Some(entry) = self.entries.get(path) {
            if entry.is_valid(path, &self.algorithm) {
                log::debug!("Cache hit for file: {:?}", path);
                return Some(entry.to_file_info());
            }
        }

        None
    }

    /// Store a file hash in the cache
    pub fn store(&mut self, file_info: &FileInfo, algorithm: &str) -> Result<()> {
        if let Some(entry) = FileCacheEntry::from_file_info(file_info, algorithm) {
            self.entries.insert(file_info.path.clone(), entry);
            self.modified = true;
            Ok(())
        } else {
            Err(anyhow::anyhow!(
                "Cannot cache file without hash: {:?}",
                file_info.path
            ))
        }
    }

    /// Store multiple file infos in the cache
    pub fn store_batch(&mut self, file_infos: &[FileInfo], algorithm: &str) -> Result<usize> {
        let mut stored_count = 0;

        for file_info in file_infos {
            if let Some(entry) = FileCacheEntry::from_file_info(file_info, algorithm) {
                self.entries.insert(file_info.path.clone(), entry);
                stored_count += 1;
                self.modified = true;
            }
        }

        if stored_count > 0 {
            log::debug!("Stored {} file hashes in cache", stored_count);
        }

        Ok(stored_count)
    }

    /// Save the cache to disk
    pub fn save(&mut self) -> Result<()> {
        // Only save if the cache was modified
        if !self.modified {
            log::debug!("Cache not modified, skipping save.");
            return Ok(());
        }

        let cache_file = Self::cache_file_path(&self.cache_dir, &self.algorithm);

        // Create a temp file first
        let temp_file = cache_file.with_extension("temp");
        let mut file = File::create(&temp_file)
            .with_context(|| format!("Failed to create temporary cache file: {:?}", temp_file))?;

        let json =
            serde_json::to_vec(&self.entries).context("Failed to serialize cache entries")?;

        file.write_all(&json)
            .with_context(|| format!("Failed to write cache data to file: {:?}", temp_file))?;

        // Make sure it's flushed to disk
        file.flush()?;
        drop(file);

        // Rename the temp file to the actual cache file
        fs::rename(&temp_file, &cache_file).with_context(|| {
            format!(
                "Failed to rename temp cache file: {:?} to {:?}",
                temp_file, cache_file
            )
        })?;

        log::info!(
            "Saved {} entries to cache file: {:?}",
            self.entries.len(),
            cache_file
        );

        self.modified = false;
        Ok(())
    }

    /// Clear the cache
    pub fn clear(&mut self) {
        self.entries.clear();
        self.modified = true;
    }

    /// Get the number of entries in the cache
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Check if the cache is empty
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

impl Drop for FileCache {
    fn drop(&mut self) {
        // Try to save the cache when the object is dropped
        if self.modified {
            if let Err(e) = self.save() {
                log::error!("Failed to save cache on drop: {}", e);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use tempfile::tempdir;

    fn create_test_file(dir: &Path, name: &str, content: &[u8]) -> Result<FileInfo> {
        let file_path = dir.join(name);
        let mut file = File::create(&file_path)?;
        file.write_all(content)?;
        file.flush()?;

        let metadata = fs::metadata(&file_path)?;

        Ok(FileInfo {
            path: file_path,
            size: metadata.len(),
            hash: Some("test_hash".to_string()),
            modified_at: metadata.modified().ok(),
            created_at: metadata.created().ok(),
        })
    }

    #[test]
    fn test_cache_storage_and_retrieval() -> Result<()> {
        let temp_dir = tempdir()?;
        let cache_dir = temp_dir.path().join("cache");
        let test_dir = temp_dir.path().join("test_files");
        fs::create_dir_all(&test_dir)?;

        // Create a test file
        let test_file = create_test_file(&test_dir, "test1.txt", b"hello world")?;

        // Create a cache and store the file
        let mut cache = FileCache::new(&cache_dir, "test_algo")?;
        cache.store(&test_file, "test_algo")?;

        // Check that we can get the hash
        let hash = cache.get_hash(&test_file.path);
        assert_eq!(hash, Some("test_hash".to_string()));

        // Save and recreate the cache
        cache.save()?;
        drop(cache);

        // Load the cache again and check the hash is still there
        let cache2 = FileCache::new(&cache_dir, "test_algo")?;
        let hash2 = cache2.get_hash(&test_file.path);
        assert_eq!(hash2, Some("test_hash".to_string()));

        Ok(())
    }

    #[test]
    fn test_cache_invalidation() -> Result<()> {
        let temp_dir = tempdir()?;
        let cache_dir = temp_dir.path().join("cache");
        let test_dir = temp_dir.path().join("test_files");
        fs::create_dir_all(&test_dir)?;

        // Create a test file
        let test_file = create_test_file(&test_dir, "test1.txt", b"hello world")?;

        // Create a cache and store the file
        let mut cache = FileCache::new(&cache_dir, "test_algo")?;
        cache.store(&test_file, "test_algo")?;
        cache.save()?;

        // Modify the file
        std::thread::sleep(std::time::Duration::from_millis(100)); // Ensure modification time changes
        let mut file = File::create(&test_file.path)?;
        file.write_all(b"changed content")?;
        file.flush()?;

        // Hash should not be valid anymore
        let hash = cache.get_hash(&test_file.path);
        assert_eq!(hash, None);

        Ok(())
    }

    #[test]
    fn test_algorithm_mismatch() -> Result<()> {
        let temp_dir = tempdir()?;
        let cache_dir = temp_dir.path().join("cache");
        let test_dir = temp_dir.path().join("test_files");
        fs::create_dir_all(&test_dir)?;

        // Create a test file
        let test_file = create_test_file(&test_dir, "test1.txt", b"hello world")?;

        // Create a cache with one algorithm
        let mut cache = FileCache::new(&cache_dir, "algo1")?;
        cache.store(&test_file, "algo1")?;
        cache.save()?;

        // Try to get the hash with a different algorithm
        let cache2 = FileCache::new(&cache_dir, "algo2")?;
        let hash = cache2.get_hash(&test_file.path);
        assert_eq!(hash, None);

        Ok(())
    }
}
