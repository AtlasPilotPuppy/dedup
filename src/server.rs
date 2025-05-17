#[cfg(feature = "ssh")]
use crate::options::DedupOptions;
#[cfg(feature = "ssh")]
use crate::protocol::{
    create_protocol_handler, CommandMessage, DedupMessage, ErrorMessage, MessageType,
    ProtocolHandler, ResultMessage,
};
#[cfg(feature = "ssh")]
use anyhow::{anyhow, Context, Result};
#[cfg(feature = "ssh")]
use log;
#[cfg(feature = "ssh")]
use serde_json;
#[cfg(feature = "ssh")]
use std::io::{BufRead, BufReader};
#[cfg(feature = "ssh")]
use std::net::{TcpListener, TcpStream};
#[cfg(feature = "ssh")]
use std::process::{Command, Stdio};
#[cfg(feature = "ssh")]
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc, Mutex,
};
#[cfg(feature = "ssh")]
use std::thread;
#[cfg(feature = "ssh")]
use std::time::Duration;

/// Server implementation that listens for client commands and executes dedups operations
#[cfg(feature = "ssh")]
pub struct DedupServer {
    port: u16,
    running: Arc<AtomicBool>,
    options: Arc<Mutex<DedupOptions>>,
}

#[cfg(feature = "ssh")]
impl DedupServer {
    pub fn new(port: u16) -> Self {
        Self {
            port,
            running: Arc::new(AtomicBool::new(false)),
            options: Arc::new(Mutex::new(DedupOptions::default())),
        }
    }

    pub fn with_options(port: u16, options: DedupOptions) -> Self {
        Self {
            port,
            running: Arc::new(AtomicBool::new(false)),
            options: Arc::new(Mutex::new(options)),
        }
    }

    /// Start the server on the given port
    pub fn start(&mut self) -> Result<()> {
        let listener = TcpListener::bind(format!("127.0.0.1:{}", self.port))
            .with_context(|| format!("Failed to bind to port {} for dedups server", self.port))?;

        listener
            .set_nonblocking(true)
            .context("Failed to set listener to non-blocking mode")?;

        self.running.store(true, Ordering::SeqCst);
        let running = self.running.clone();
        let options = self.options.clone();

        log::info!("Dedups server started on port {}", self.port);

        while running.load(Ordering::SeqCst) {
            match listener.accept() {
                Ok((stream, addr)) => {
                    log::info!("New client connection from: {}", addr);
                    // Handle client in a new thread
                    let running_clone = running.clone();
                    let options_clone = options.clone();
                    thread::spawn(move || {
                        if let Err(e) = Self::handle_client(stream, running_clone, options_clone) {
                            log::error!("Error handling client: {}", e);
                        }
                    });
                }
                Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                    // No connection available, sleep briefly
                    thread::sleep(Duration::from_millis(100));
                }
                Err(e) => {
                    log::error!("Error accepting connection: {}", e);
                    break;
                }
            }
        }

        log::info!("Dedups server stopped");
        Ok(())
    }

    /// Stop the server
    pub fn stop(&self) {
        self.running.store(false, Ordering::SeqCst);
        log::info!("Server shutdown initiated");
    }

    /// Handle a client connection
    fn handle_client(
        stream: TcpStream,
        running: Arc<AtomicBool>,
        options: Arc<Mutex<DedupOptions>>,
    ) -> Result<()> {
        // Get protocol options from shared state
        let opts = options
            .lock()
            .map_err(|_| anyhow!("Failed to lock options"))?;

        let use_protobuf = {
            #[cfg(feature = "proto")]
            {
                opts.use_protobuf
            }
            #[cfg(not(feature = "proto"))]
            {
                false
            }
        };

        let use_compression = {
            #[cfg(feature = "proto")]
            {
                opts.use_compression
            }
            #[cfg(not(feature = "proto"))]
            {
                false
            }
        };

        let compression_level = {
            #[cfg(feature = "proto")]
            {
                opts.compression_level
            }
            #[cfg(not(feature = "proto"))]
            {
                3
            }
        };

        drop(opts); // Release the lock

        let mut protocol =
            create_protocol_handler(stream, use_protobuf, use_compression, compression_level)?;

        while running.load(Ordering::SeqCst) {
            if let Some(message) = protocol.receive_message()? {
                match message.message_type {
                    MessageType::Command => {
                        Self::handle_command(&mut *protocol, &message)?;
                    }
                    _ => {
                        log::warn!(
                            "Received unexpected message type: {:?}",
                            message.message_type
                        );
                        Self::send_error(
                            &mut *protocol,
                            "Unexpected message type, expected command",
                            1,
                        )?;
                    }
                }
            } else {
                // EOF, client disconnected
                log::info!("Client disconnected");
                break;
            }
        }

        Ok(())
    }

    /// Handle a command message
    fn handle_command(protocol: &mut dyn ProtocolHandler, message: &DedupMessage) -> Result<()> {
        let command_msg: CommandMessage = serde_json::from_str(&message.payload)?;

        log::info!(
            "Executing command: {} with {} args",
            command_msg.command,
            command_msg.args.len()
        );

        // Build and execute the command
        let mut cmd = Command::new(&command_msg.command);
        cmd.args(&command_msg.args);
        cmd.stdout(Stdio::piped());
        cmd.stderr(Stdio::piped());

        // Add environment variables from options
        for (key, value) in &command_msg.options {
            if key.starts_with("ENV_") {
                let env_name = key.trim_start_matches("ENV_");
                cmd.env(env_name, value);
            }
        }

        // Execute the command
        match cmd.spawn() {
            Ok(mut child) => {
                let stdout = child.stdout.take().expect("Failed to open stdout");
                let stderr = child.stderr.take().expect("Failed to open stderr");

                // Process stdout in a separate thread
                let mut protocol_clone = protocol.box_clone();
                let stdout_thread = thread::spawn(move || {
                    let reader = BufReader::new(stdout);
                    for line in reader.lines() {
                        if let Ok(line) = line {
                            // Try to parse as JSON
                            if line.starts_with('{') && line.ends_with('}') {
                                // Forward as Result message
                                let result_msg = DedupMessage {
                                    message_type: MessageType::Result,
                                    payload: line.clone(),
                                };

                                if let Err(e) = protocol_clone.send_message(result_msg) {
                                    log::error!("Error sending output to client: {}", e);
                                    break;
                                }
                            } else {
                                log::debug!("STDOUT: {}", line);
                            }
                        }
                    }
                });

                // Process stderr in main thread
                let stderr_reader = BufReader::new(stderr);
                for line in stderr_reader.lines() {
                    if let Ok(line) = line {
                        log::debug!("STDERR: {}", line);
                    }
                }

                // Wait for stdout thread to complete
                stdout_thread.join().expect("Stdout thread panicked");

                // Wait for process to exit
                let status = child.wait()?;
                log::info!("Command exited with status: {}", status);

                // Send final result message
                let result = ResultMessage {
                    duplicate_count: 0, // These would come from actual output parsing
                    total_files: 0,
                    total_bytes: 0,
                    duplicate_bytes: 0,
                    elapsed_seconds: 0.0,
                };

                let result_json = serde_json::to_string(&result)?;
                let result_msg = DedupMessage {
                    message_type: MessageType::Result,
                    payload: result_json,
                };

                protocol.send_message(result_msg)?;

                Ok(())
            }
            Err(e) => {
                log::error!("Failed to execute command: {}", e);
                Self::send_error(protocol, &format!("Failed to execute command: {}", e), 2)
            }
        }
    }

    /// Send an error message to the client
    fn send_error(protocol: &mut dyn ProtocolHandler, message: &str, code: i32) -> Result<()> {
        let error = ErrorMessage {
            message: message.to_string(),
            code,
        };

        let error_json = serde_json::to_string(&error)?;
        let error_msg = DedupMessage {
            message_type: MessageType::Error,
            payload: error_json,
        };

        protocol.send_message(error_msg)?;
        Ok(())
    }
}

/// Entry point for running the server
#[cfg(feature = "ssh")]
pub fn run_server(port: u16) -> Result<()> {
    let mut server = DedupServer::new(port);
    server.start()?;
    Ok(())
}

/// Entry point for running the server with custom options
#[cfg(feature = "ssh")]
pub fn run_server_with_options(port: u16, options: DedupOptions) -> Result<()> {
    let mut server = DedupServer::with_options(port, options);
    server.start()?;
    Ok(())
}

/// Check if the server is already running
#[cfg(feature = "ssh")]
pub fn check_server_running(port: u16) -> bool {
    TcpStream::connect(format!("127.0.0.1:{}", port)).is_ok()
}
