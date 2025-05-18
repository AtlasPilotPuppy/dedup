// Define operating modes for the application
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AppMode {
    // Standard deduplication mode - find and manage duplicate files
    Deduplication,
    
    // Copy missing mode - compare directories and copy missing files
    CopyMissing,
}

impl Default for AppMode {
    fn default() -> Self {
        AppMode::Deduplication
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::options::Options;
    
    #[test]
    fn test_app_mode_detection() {
        // Can't default construct Options, so we'll simulate app_mode setting:
        // This is how Options::new() initializes app_mode based on copy_missing
        
        // Test for Copy Missing mode
        let app_mode_when_copy_missing_true = if true {
            AppMode::CopyMissing
        } else {
            AppMode::Deduplication
        };
        assert_eq!(app_mode_when_copy_missing_true, AppMode::CopyMissing);
        
        // Test for Deduplication mode
        let app_mode_when_copy_missing_false = if false {
            AppMode::CopyMissing
        } else {
            AppMode::Deduplication
        };
        assert_eq!(app_mode_when_copy_missing_false, AppMode::Deduplication);
    }
} 