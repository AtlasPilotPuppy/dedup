#[cfg(all(test, feature = "ssh", feature = "proto"))]
mod docker_tests {
    use std::process::Command;
    use std::path::Path;

    #[test]
    #[ignore] // This test requires Docker, so we'll mark it as ignored by default
    fn test_ssh_api_communication() {
        let tests_path = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/docker");
        
        // Change to the docker directory
        assert!(std::env::set_current_dir(&tests_path).is_ok());
        
        // Clean up any existing containers
        let _ = Command::new("docker-compose")
            .args(&["down", "-v"])
            .output();
            
        // Build and run the tests
        let output = Command::new("docker-compose")
            .args(&["up", "--build", "--abort-on-container-exit"])
            .output()
            .expect("Failed to execute docker-compose command");
            
        // Clean up
        let _ = Command::new("docker-compose")
            .args(&["down", "-v"])
            .output();
        
        // Output for debugging
        println!("=== Docker Test Output ===");
        println!("{}", String::from_utf8_lossy(&output.stdout));
        println!("=== Docker Test Error Output ===");
        println!("{}", String::from_utf8_lossy(&output.stderr));
        
        // Check if test was successful by examining output
        let output_str = String::from_utf8_lossy(&output.stdout);
        assert!(output_str.contains("All tests completed successfully!"), 
                "Docker test failed! Check output above.");
    }
} 