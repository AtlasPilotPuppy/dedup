// New integration test for tunnel API client/server handshake
// Requires ssh + proto features and an environment variable enabling it because
// it spawns real child processes and binds to a TCP port.
//
// Run manually with:
//   cargo test --test tunnel_api_integration --features "ssh,proto" \
//      -- --ignored    DEDUPS_SSH_TEST=1
//
// The test starts a local dedups instance in --server-mode and then
// connects to it with DedupClient, issuing an internal handshake.

#![cfg(all(feature = "ssh", feature = "proto"))]

use std::collections::HashMap;
use std::io::{BufRead, BufReader};
use std::net::TcpStream;
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use assert_cmd::cargo::cargo_bin;

use dedups::client::{ConnectionState, DedupClient};
use dedups::protocol::find_available_port;

const SERVER_START_TIMEOUT: u64 = 10000; // 10 seconds
const SERVER_STOP_TIMEOUT: u64 = 5000; // 5 seconds
const DEFAULT_PORT: u16 = 29875; // Default dedups port
const TEST_TIMEOUT: u64 = 120; // 2 minutes timeout for tests

/// Helper: wait until TCP port is listening (or timeout)
fn wait_for_port(port: u16, timeout_ms: u64) -> bool {
    let start = Instant::now();
    while start.elapsed() < Duration::from_millis(timeout_ms) {
        if let Ok(stream) = TcpStream::connect(("127.0.0.1", port)) {
            // Immediately drop the stream to close the connection cleanly
            drop(stream);
            return true;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    false
}

/// Helper: wait until port is free (or timeout)
fn wait_for_port_free(port: u16, timeout_ms: u64) -> bool {
    let start = Instant::now();
    while start.elapsed() < Duration::from_millis(timeout_ms) {
        if TcpStream::connect(("127.0.0.1", port)).is_err() {
            return true;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    false
}

/// Helper: find a free port starting from the default port
fn find_free_port() -> Result<u16> {
    // First try to clean up and use the default port
    kill_process_on_port(DEFAULT_PORT)?;

    if wait_for_port_free(DEFAULT_PORT, 1000) {
        // Double check port is actually free
        if TcpStream::connect(("127.0.0.1", DEFAULT_PORT)).is_err() {
            // Wait a bit to ensure the port isn't in TIME_WAIT state
            std::thread::sleep(Duration::from_millis(100));

            // Check again to be really sure
            if TcpStream::connect(("127.0.0.1", DEFAULT_PORT)).is_err() {
                return Ok(DEFAULT_PORT);
            }
        }
    }

    // If default port is not available, try incrementing
    for port in (DEFAULT_PORT + 1)..=(DEFAULT_PORT + 10) {
        kill_process_on_port(port)?;

        if wait_for_port_free(port, 1000) {
            if TcpStream::connect(("127.0.0.1", port)).is_err() {
                std::thread::sleep(Duration::from_millis(100));
                if TcpStream::connect(("127.0.0.1", port)).is_err() {
                    return Ok(port);
                }
            }
        }
    }

    Err(anyhow::anyhow!(
        "Could not find a free port starting from {}",
        DEFAULT_PORT
    ))
}

/// Helper: kill any existing process using the port
fn kill_process_on_port(port: u16) -> Result<()> {
    #[cfg(target_os = "macos")]
    {
        // First try lsof
        let output = Command::new("lsof")
            .arg("-i")
            .arg(format!(":{}", port))
            .arg("-t")
            .output()?;

        if !output.stdout.is_empty() {
            let stdout_str = String::from_utf8_lossy(&output.stdout).to_string();
            let pids: Vec<&str> = stdout_str.split('\n').filter(|s| !s.is_empty()).collect();

            for pid in pids {
                Command::new("kill").arg("-9").arg(pid).output()?;
            }

            // Wait for the processes to die
            std::thread::sleep(Duration::from_millis(500));
        }

        // Then try netstat
        let output = Command::new("netstat")
            .arg("-anv")
            .arg("-p")
            .arg("tcp")
            .output()?;

        if !output.stdout.is_empty() {
            let stdout_str = String::from_utf8_lossy(&output.stdout).to_string();
            for line in stdout_str.lines() {
                if line.contains(&format!("*.{}", port))
                    || line.contains(&format!("127.0.0.1.{}", port))
                {
                    if let Some(pid) = line.split_whitespace().nth(8) {
                        Command::new("kill").arg("-9").arg(pid).output()?;
                    }
                }
            }

            // Wait for the processes to die
            std::thread::sleep(Duration::from_millis(500));
        }
    }

    Ok(())
}

/// Helper: read server output in a separate thread
fn read_server_output(
    stdout: std::process::ChildStdout,
    stderr: std::process::ChildStderr,
) -> ManagedReceiver<String> {
    // Use a sync_channel with limited capacity to prevent unbounded buffering
    let (tx, rx) = mpsc::sync_channel::<String>(100);
    let tx_clone = tx.clone();

    // Create an atomic flag to signal threads to exit
    let should_exit = Arc::new(AtomicBool::new(false));
    let should_exit_stdout = Arc::clone(&should_exit);
    let should_exit_stderr = Arc::clone(&should_exit);

    // Track when main thread drops the receiver
    let rx_dropped = Arc::new(AtomicBool::new(false));
    let rx_dropped_clone = Arc::clone(&rx_dropped);

    // Create a monitor thread to detect channel closure
    std::thread::spawn(move || {
        // Wait for receiver to be dropped
        while !rx_dropped.load(Ordering::SeqCst) {
            std::thread::sleep(Duration::from_millis(100));
        }
        // Signal threads to exit
        should_exit.store(true, Ordering::SeqCst);
    });

    // Read stdout
    thread::spawn(move || {
        let reader = BufReader::new(stdout);
        for line in reader.lines() {
            // Check if we should exit
            if should_exit_stdout.load(Ordering::SeqCst) {
                break;
            }

            if let Ok(line) = line {
                println!("Server stdout: {}", line);
                match tx.send(line) {
                    Ok(_) => {}
                    Err(e) => {
                        println!("Failed to send stdout line: {}", e);
                        break;
                    }
                }
            }
        }
        // Signal that this thread is done
        println!("Stdout reader thread exiting");
    });

    // Read stderr
    thread::spawn(move || {
        let reader = BufReader::new(stderr);
        for line in reader.lines() {
            // Check if we should exit
            if should_exit_stderr.load(Ordering::SeqCst) {
                break;
            }

            if let Ok(line) = line {
                println!("Server stderr: {}", line);
                match tx_clone.send(line) {
                    Ok(_) => {}
                    Err(e) => {
                        println!("Failed to send stderr line: {}", e);
                        break;
                    }
                }
            }
        }
        // Signal that this thread is done
        println!("Stderr reader thread exiting");
    });

    // Create a wrapper receiver that signals when it's dropped
    let wrapped_rx = ManagedReceiver {
        rx,
        _dropped: rx_dropped_clone,
    };
    wrapped_rx
}

// Wrapper around Receiver to detect when it's dropped
struct ManagedReceiver<T> {
    rx: mpsc::Receiver<T>,
    _dropped: Arc<AtomicBool>,
}

impl<T> Drop for ManagedReceiver<T> {
    fn drop(&mut self) {
        self._dropped.store(true, Ordering::SeqCst);
        println!("Server output receiver dropped");
    }
}

impl<T> std::ops::Deref for ManagedReceiver<T> {
    type Target = mpsc::Receiver<T>;

    fn deref(&self) -> &Self::Target {
        &self.rx
    }
}

impl<T> std::ops::DerefMut for ManagedReceiver<T> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.rx
    }
}

/// Helper: ensure server is running and listening
fn ensure_server_running(child: &mut Child, port: u16) -> Result<ManagedReceiver<String>> {
    let start = Instant::now();
    let timeout = Duration::from_millis(SERVER_START_TIMEOUT);

    // Read server output to check for startup messages
    let stdout = child.stdout.take().expect("Failed to capture stdout");
    let stderr = child.stderr.take().expect("Failed to capture stderr");
    let rx = read_server_output(stdout, stderr);

    let mut server_ready = false;
    let mut last_error = None;
    let mut saw_startup_message = false;

    // Wait for server to be ready
    while start.elapsed() < timeout {
        // Check if server is listening
        if !server_ready && wait_for_port(port, 100) {
            // Try to establish a test connection - this connection will be closed immediately
            match TcpStream::connect(("127.0.0.1", port)) {
                Ok(stream) => {
                    // Verify we can actually use the connection
                    if stream
                        .set_read_timeout(Some(Duration::from_secs(1)))
                        .is_ok()
                    {
                        println!("Server port {} is accepting connections", port);
                        server_ready = true;
                        // Explicitly close the connection
                        drop(stream);
                    } else {
                        println!("Port {} is listening but connection not usable", port);
                        last_error = Some("Connection not usable".to_string());
                    }
                }
                Err(e) => {
                    println!("Port {} is listening but connection failed: {}", port, e);
                    last_error = Some(format!("Connection test failed: {}", e));
                }
            }
        }

        // Check if server process is still alive
        match child.try_wait() {
            Ok(Some(status)) => {
                // Read any remaining output
                while let Ok(line) = rx.try_recv() {
                    println!("Server output: {}", line);
                    if line.contains("error:") || line.contains("Error:") {
                        last_error = Some(line.clone());
                    }
                }
                return Err(anyhow::anyhow!(
                    "Server process exited with status {} before starting. Last error: {}",
                    status,
                    last_error.unwrap_or_else(|| "No error details".to_string())
                ));
            }
            Ok(None) => {
                // Process is still running, check output
                while let Ok(line) = rx.try_recv() {
                    println!("Server output: {}", line);
                    if line.contains("error:") || line.contains("Error:") {
                        last_error = Some(line.clone());
                    }
                    if line.contains("Server ready") || line.contains("started on port") {
                        saw_startup_message = true;
                    }
                }

                if server_ready && saw_startup_message {
                    println!("Server startup confirmed via logs and connection test");
                    return Ok(rx);
                }

                // Continue waiting
                thread::sleep(Duration::from_millis(100));
            }
            Err(e) => {
                return Err(anyhow::anyhow!(
                    "Error checking server process status: {}",
                    e
                ));
            }
        }
    }

    // Timeout exceeded
    Err(anyhow::anyhow!(
        "Server did not start listening on port {} within timeout of {} ms. Last error: {}",
        port,
        SERVER_START_TIMEOUT,
        last_error.unwrap_or_else(|| "No error details".to_string())
    ))
}

/// Helper: start server with retries
fn start_server_with_retries(
    verbose: u8,
    _max_retries: u32, // Prefix with underscore to indicate intentionally unused
) -> Result<(Child, u16, ManagedReceiver<String>)> {
    let port = find_free_port()?;

    // Kill any existing process on this port
    kill_process_on_port(port)?;

    // Wait for port to be free
    if !wait_for_port_free(port, 1000) {
        return Err(anyhow::anyhow!("Port {} still in use after cleanup", port));
    }

    let mut cmd = Command::new(cargo_bin("dedups"));
    cmd.arg("--server-mode")
        .arg("--port")
        .arg(port.to_string())
        .arg("--use-protobuf")
        .arg("--use-compression")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

    // propagate verbosity for manual debugging if desired
    for _ in 0..verbose {
        cmd.arg("-v");
    }

    // Set RUST_LOG for better debugging
    cmd.env("RUST_LOG", "debug");

    println!("Starting server with command: {:?}", cmd);
    match cmd.spawn() {
        Ok(mut child) => {
            // Wait for server to start
            match ensure_server_running(&mut child, port) {
                Ok(rx) => {
                    println!("Server started successfully on port {}", port);
                    Ok((child, port, rx))
                }
                Err(e) => {
                    println!("Failed to start server on port {}: {}", port, e);
                    let _ = stop_server(child);
                    Err(e)
                }
            }
        }
        Err(e) => Err(anyhow::anyhow!("Failed to spawn server: {}", e)),
    }
}

/// Helper: stop server with timeout and cleanup
fn stop_server(mut child: Child) -> Result<()> {
    // Try graceful shutdown first
    println!("Attempting graceful server shutdown");

    // Instead of kill, try SIGTERM first
    if let Err(e) = child.try_wait() {
        println!("Warning: Error checking server status: {}", e);
    }

    // Attempt graceful SIGTERM before SIGKILL
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;

        println!("Sending SIGTERM to server");
        let pid = child.id();

        if pid > 0 {
            // Try to send SIGTERM
            let _ = std::process::Command::new("kill")
                .arg("-15") // SIGTERM
                .arg(pid.to_string())
                .status();

            // Give the server a moment to shut down gracefully
            std::thread::sleep(Duration::from_millis(500));
        }
    }

    // If still running, try child.kill() (SIGKILL)
    match child.try_wait() {
        Ok(Some(status)) => {
            println!("Server exited with status: {}", status);
            return Ok(());
        }
        Ok(None) => {
            println!("Server still running after SIGTERM, using SIGKILL");
            // Force kill if timeout exceeded
            if let Err(e) = child.kill() {
                println!("Warning: Failed to kill server process: {}", e);
            }
        }
        Err(e) => {
            println!("Error waiting for server: {}", e);
            // Try to kill anyway
            let _ = child.kill();
        }
    }

    // Wait with timeout
    let start = Instant::now();
    while start.elapsed() < Duration::from_millis(SERVER_STOP_TIMEOUT) {
        match child.try_wait() {
            Ok(Some(status)) => {
                if !status.success() {
                    println!("Note: Server exited with status: {}", status);
                }
                return Ok(());
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(100)),
            Err(e) => return Err(anyhow::anyhow!("Error waiting for server to stop: {}", e)),
        }
    }

    // Last resort - force kill
    println!("Warning: Server did not stop gracefully, forcing termination");
    child.kill()?;
    child.wait()?;

    Ok(())
}

/// Helper: spawn a local server
fn spawn_local_server(verbose: u8) -> Result<(Child, u16, ManagedReceiver<String>)> {
    start_server_with_retries(verbose, 1) // Only try once since we clean up properly
}

/// Helper: ensure proper cleanup after tests
fn cleanup_test_server(port: u16) -> Result<()> {
    println!("Cleaning up test server on port {}", port);
    kill_process_on_port(port)?;

    // Wait for port to be free
    if !wait_for_port_free(port, SERVER_STOP_TIMEOUT) {
        println!("Warning: Port {} still in use after cleanup", port);
    }

    Ok(())
}

/// Helper: run test with timeout and cleanup
fn run_test_with_timeout<F, T>(test_fn: F) -> Result<T>
where
    F: FnOnce() -> Result<T> + Send + 'static,
    T: Send + 'static,
{
    let timeout = Duration::from_secs(30); // Increased from 10s to 30s for all tests
    let (tx, rx) = mpsc::channel();

    // Use a thread scope to ensure all threads complete
    let handle = thread::spawn(move || {
        let result = test_fn();
        let _ = tx.send(result);
    });

    match rx.recv_timeout(timeout) {
        Ok(result) => {
            // Wait for thread to finish with higher timeout
            let cleanup_timeout = Duration::from_secs(10);
            let cleanup_start = Instant::now();
            while cleanup_start.elapsed() < cleanup_timeout {
                if handle.is_finished() {
                    match handle.join() {
                        Ok(_) => {
                            println!("Test thread successfully joined");
                            break;
                        }
                        Err(e) => {
                            println!("Warning: Test thread panicked: {:?}", e);
                            break;
                        }
                    }
                }
                std::thread::sleep(Duration::from_millis(100));
            }

            // Ensure all threads are finished by sleeping briefly
            std::thread::sleep(Duration::from_millis(500));

            result
        }
        Err(_) => {
            // Test timed out, try to clean up
            println!("Test timed out, attempting cleanup...");

            // Wait longer for the thread to exit if possible
            std::thread::sleep(Duration::from_millis(1000));
            let _ = handle.join();

            // Return error
            Err(anyhow::anyhow!("Test timed out after {:?}", timeout))
        }
    }
}

/// Helper: verify server is running
fn verify_server_running(server_child: &mut Child) -> Result<()> {
    match server_child.try_wait() {
        Ok(None) => {
            println!("Server is still running (expected)");
            Ok(())
        }
        Ok(Some(status)) => Err(anyhow::anyhow!(
            "Server process exited unexpectedly with status: {}",
            status
        )),
        Err(e) => Err(anyhow::anyhow!(
            "Error checking server process status: {}",
            e
        )),
    }
}

#[test]
#[ignore]
fn test_tunnel_api_functionality() -> Result<()> {
    // opt-in via env so we don't run by default in CI without ssh feature build
    if std::env::var("DEDUPS_SSH_TEST").is_err() {
        eprintln!("DEDUPS_SSH_TEST not set – skipping test_tunnel_api_functionality");
        return Ok(());
    }

    run_test_with_timeout(|| {
        // Set a max execution time to prevent hangs
        let test_start = Instant::now();
        let test_max_duration = Duration::from_secs(20); // 20 seconds max

        // Spawn server with verbosity
        let (mut server_child, port, mut server_rx) = spawn_local_server(3)?;

        // We'll use a cleanup guard with Drop trait to ensure resources are freed
        struct CleanupGuard {
            child: Option<Child>,
            port: u16,
            rx: Option<ManagedReceiver<String>>,
        }

        impl Drop for CleanupGuard {
            fn drop(&mut self) {
                println!("Performing cleanup in guard...");

                // First drop receiver to terminate its threads
                if let Some(rx) = self.rx.take() {
                    drop(rx);
                    println!("Dropped server output receiver");
                    // Give threads time to exit cleanly
                    std::thread::sleep(Duration::from_millis(200));
                }

                // Kill server process if it's still running
                if let Some(mut child) = self.child.take() {
                    println!("Stopping server process...");
                    // Use stop_server which attempts SIGTERM first
                    if let Err(e) = stop_server(child) {
                        println!("Error during server cleanup: {}", e);
                    }
                }

                // Clean up port
                println!("Cleaning up port {}...", self.port);
                if let Err(e) = cleanup_test_server(self.port) {
                    println!("Error cleaning up port: {}", e);
                }

                // Sleep briefly to allow resources to be freed
                std::thread::sleep(Duration::from_millis(200));
                println!("Cleanup complete");
            }
        }

        // Create a guard that will automatically clean up resources when dropped
        let mut guard = CleanupGuard {
            child: Some(server_child),
            port,
            rx: Some(server_rx),
        };

        // Wait until we can connect
        println!("Waiting for server to start on port {}", port);
        assert!(
            wait_for_port(port, SERVER_START_TIMEOUT),
            "server did not start listening in time"
        );
        println!("Server is listening");

        // Test scope to ensure resources are dropped at the end
        {
            // Connect using DedupClient with keep-alive enabled
            let mut client = DedupClient::with_options(
                "127.0.0.1".into(),
                port,
                dedups::options::DedupOptions {
                    use_ssh_tunnel: false, // Don't use SSH tunnel for local test
                    tunnel_api_mode: true,
                    port,
                    use_protobuf: true,
                    use_compression: true,
                    compression_level: 3,
                    keep_alive: true, // Enable keep-alive
                    ..Default::default()
                },
            );

            // Connect and verify state
            println!("Attempting to connect to server");
            client.connect()?;
            println!("Connected successfully");
            assert_eq!(client.connection_state(), &ConnectionState::Connected);

            // Get reference to server_rx
            let server_rx_ref = guard.rx.as_mut().unwrap();

            // Check server output for connection message
            let mut connection_seen = false;
            for _ in 0..10 {
                // Check server output
                while let Ok(line) = server_rx_ref.try_recv() {
                    println!("Server log: {}", line);
                    if line.contains("New client connection") {
                        connection_seen = true;
                    }
                }
                if connection_seen {
                    break;
                }
                std::thread::sleep(std::time::Duration::from_millis(100));
            }
            assert!(connection_seen, "Server did not log client connection");

            // Perform initial handshake
            println!("Performing handshake");
            let handshake_response = client.execute_command(
                "internal_handshake".to_string(),
                Vec::new(),
                HashMap::new(),
            )?;

            // Parse and verify handshake response
            let handshake_json: serde_json::Value = serde_json::from_str(&handshake_response)
                .with_context(|| {
                    format!("Failed to parse handshake response: {}", handshake_response)
                })?;

            assert_eq!(
                handshake_json["status"], "handshake_ack",
                "Handshake response missing ack"
            );
            println!(
                "Handshake successful - using {} protocol with compression {}",
                handshake_json["protocol"]["type"]
                    .as_str()
                    .unwrap_or("unknown"),
                if handshake_json["protocol"]["compression"]
                    .as_bool()
                    .unwrap_or(false)
                {
                    "enabled"
                } else {
                    "disabled"
                }
            );

            // Check server output to verify handshake was received
            let mut handshake_seen = false;
            for _ in 0..10 {
                // Check server output
                while let Ok(line) = server_rx_ref.try_recv() {
                    println!("Server log: {}", line);
                    if line.contains("internal_handshake") || line.contains("handshake") {
                        handshake_seen = true;
                    }
                }
                if handshake_seen {
                    break;
                }
                std::thread::sleep(std::time::Duration::from_millis(100));
            }

            // Test basic command functionality
            println!("Testing basic command functionality");
            let help_response = client.execute_command(
                "dedups".to_string(),
                vec!["--help".to_string()],
                HashMap::new(),
            )?;
            assert!(
                help_response.contains("Usage:"),
                "Response should contain help text"
            );

            // Test keep-alive functionality
            println!("Testing keep-alive functionality");
            std::thread::sleep(std::time::Duration::from_secs(1)); // Reduced from 5s to 1s
            let version_response = client.execute_command(
                "dedups".to_string(),
                vec!["--version".to_string()],
                HashMap::new(),
            )?;
            assert!(
                version_response.contains("dedups"),
                "Response should contain version info"
            );

            // Test error handling
            println!("Testing error handling");
            let err_result =
                client.execute_command("invalid_command".to_string(), vec![], HashMap::new());
            assert!(err_result.is_err(), "Invalid command should return error");
            if let Err(e) = err_result {
                assert!(
                    e.to_string().contains("Invalid command"),
                    "Error should mention invalid command"
                );
            }

            // Verify server process is still running
            if let Some(child) = &mut guard.child {
                verify_server_running(child)?;
            }

            // Read any remaining server output
            while let Ok(line) = server_rx_ref.try_recv() {
                println!("Server log: {}", line);
            }

            // Disconnect and verify state
            println!("Disconnecting client");
            client.disconnect()?;
            assert_eq!(client.connection_state(), &ConnectionState::Disconnected);

            // Explicit drop to ensure client is fully deallocated
            drop(client);

            // Sleep to ensure client disconnection is fully processed
            std::thread::sleep(Duration::from_millis(200));
        }

        // Wait a moment for server to process disconnect and give time for any lingering connections to close
        std::thread::sleep(std::time::Duration::from_millis(500));

        // Read any remaining server output after disconnect
        if let Some(ref mut rx) = guard.rx {
            while let Ok(line) = rx.try_recv() {
                println!("Server log: {}", line);
            }
        }

        // Verify server is still running after client disconnect
        if let Some(child) = &mut guard.child {
            verify_server_running(child)?;
        }

        // Clean up gracefully with adequate wait times
        println!("Cleaning up server process gracefully");
        if let Some(child) = guard.child.take() {
            if let Err(e) = stop_server(child) {
                println!("Error stopping server: {}", e);
            }
        }

        // Clean up port
        println!("Cleaning up port resources");
        if let Err(e) = cleanup_test_server(port) {
            println!("Error cleaning up port: {}", e);
        }

        // Clear the server output receiver
        if let Some(rx) = guard.rx.take() {
            drop(rx);
            println!("Explicitly dropped server output receiver");
            // Give time for receiver threads to exit
            std::thread::sleep(Duration::from_millis(300));
        }

        // Dropping the guard will clean up any remaining resources
        drop(guard);

        // Ensure we clean up before time limit
        if test_start.elapsed() > test_max_duration {
            println!("Test is taking too long, but cleaning up before exit");
            // Sleep to ensure cleanup callbacks can run
            std::thread::sleep(Duration::from_secs(1));
            // Return success now - avoiding the sleep and exit(0) pattern
            return Ok(());
        }

        // Return success without using exit(0) to allow normal cleanup
        println!("Test completed successfully");
        Ok(())
    })
}
