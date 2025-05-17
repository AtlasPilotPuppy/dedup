#[cfg(feature = "ssh")]
use crate::options::DedupOptions;
#[cfg(feature = "ssh")]
use crate::protocol::{
    create_protocol_handler, CommandMessage, DedupMessage, ErrorMessage, MessageType,
    ProgressMessage, ProtocolHandler,
};
#[cfg(feature = "ssh")]
use anyhow::{anyhow, Result};
#[cfg(feature = "ssh")]
use log;
#[cfg(feature = "ssh")]
use serde_json;
#[cfg(feature = "ssh")]
use std::collections::HashMap;
#[cfg(feature = "ssh")]
use std::net::TcpStream;
#[cfg(feature = "ssh")]
use std::sync::mpsc::{self, Receiver};
#[cfg(feature = "ssh")]
use std::thread::{self, JoinHandle};

/// Client for communicating with a dedups server
#[cfg(feature = "ssh")]
pub struct DedupClient {
    host: String,
    port: u16,
    protocol: Option<Box<dyn ProtocolHandler>>,
    options: DedupOptions,
    message_receiver: Option<Receiver<DedupMessage>>,
    reader_thread: Option<JoinHandle<()>>,
    verbose: u8,
}

#[cfg(feature = "ssh")]
impl DedupClient {
    pub fn new(host: String, port: u16) -> Self {
        Self {
            host,
            port,
            protocol: None,
            options: DedupOptions::default(),
            message_receiver: None,
            reader_thread: None,
            verbose: 0,
        }
    }

    pub fn with_options(host: String, port: u16, options: DedupOptions) -> Self {
        // Initialize verbosity from environment if set
        let verbose = if let Ok(val) = std::env::var("VERBOSITY") {
            val.parse::<u8>().unwrap_or(0)
        } else {
            0
        };
        
        // Set default options to ensure Protobuf and compression are enabled
        #[cfg(feature = "proto")]
        let mut options_with_defaults = options;
        
        #[cfg(feature = "proto")]
        {
            // Set defaults for protocol options unless explicitly set to false
            if !options_with_defaults.use_protobuf {
                options_with_defaults.use_protobuf = true;
            }
            if !options_with_defaults.use_compression {
                options_with_defaults.use_compression = true;
            }
            if options_with_defaults.compression_level == 0 {
                options_with_defaults.compression_level = 3;
            }
        }
        
        #[cfg(not(feature = "proto"))]
        let options_with_defaults = options;
        
        Self {
            host,
            port,
            protocol: None,
            options: options_with_defaults,
            message_receiver: None,
            reader_thread: None,
            verbose,
        }
    }

    /// Connect to the dedups server
    pub fn connect(&mut self) -> Result<()> {
        if self.verbose >= 2 {
            log::info!("Connecting to dedups server at {}:{}", self.host, self.port);
        } else {
            log::debug!("Connecting to dedups server at {}:{}", self.host, self.port);
        }

        let stream = match TcpStream::connect(format!("{}:{}", self.host, self.port)) {
            Ok(s) => s,
            Err(e) => {
                if self.verbose >= 1 {
                    log::error!("Failed to connect to dedups server at {}:{}: {}", self.host, self.port, e);
                }
                return Err(anyhow::anyhow!("Failed to connect to dedups server at {}:{}: {}", self.host, self.port, e));
            }
        };

        // Create protocol handler based on options
        let use_protobuf = {
            #[cfg(feature = "proto")]
            {
                if self.options.use_protobuf {
                    if self.verbose >= 2 {
                        log::info!("Using Protobuf protocol for API communication");
                    }
                    true
                } else {
                    if self.verbose >= 2 {
                        log::info!("Using JSON protocol for API communication");
                    }
                    false
                }
            }
            #[cfg(not(feature = "proto"))]
            {
                if self.verbose >= 2 {
                    log::info!("Using JSON protocol for API communication (protobuf not available)");
                }
                false
            }
        };

        let use_compression = {
            #[cfg(feature = "proto")]
            {
                if self.options.use_compression && use_protobuf {
                    if self.verbose >= 3 {
                        log::debug!("Compression enabled for Protobuf communication");
                    }
                    true
                } else {
                    false
                }
            }
            #[cfg(not(feature = "proto"))]
            {
                false
            }
        };

        let compression_level = {
            #[cfg(feature = "proto")]
            {
                if self.verbose >= 3 && use_compression {
                    log::debug!("Using compression level {}", self.options.compression_level);
                }
                self.options.compression_level
            }
            #[cfg(not(feature = "proto"))]
            {
                3
            }
        };

        let protocol = match create_protocol_handler(
            stream.try_clone()?,
            use_protobuf,
            use_compression,
            compression_level,
        ) {
            Ok(p) => p,
            Err(e) => {
                if self.verbose >= 1 {
                    log::error!("Failed to create protocol handler: {}", e);
                }
                return Err(anyhow::anyhow!("Failed to create protocol handler: {}", e));
            }
        };

        // Create a message channel for receiving messages from the server
        let (tx, rx) = mpsc::channel();
        self.message_receiver = Some(rx);

        // Start a background thread to read messages from the server
        let mut reader_protocol = match create_protocol_handler(
            stream, 
            use_protobuf, 
            use_compression, 
            compression_level
        ) {
            Ok(p) => p,
            Err(e) => {
                if self.verbose >= 1 {
                    log::error!("Failed to create reader protocol handler: {}", e);
                }
                return Err(anyhow::anyhow!("Failed to create reader protocol handler: {}", e));
            }
        };
        
        let verbose = self.verbose;
        let reader_thread = thread::spawn(move || {
            if verbose >= 3 {
                log::debug!("Starting server reader thread");
            }
            
            loop {
                match reader_protocol.receive_message() {
                    Ok(Some(msg)) => {
                        if verbose >= 3 {
                            log::debug!("Received message type: {:?}", msg.message_type);
                        }
                        
                        if tx.send(msg).is_err() {
                            log::error!("Failed to send message to channel, receiver dropped");
                            break;
                        }
                    }
                    Ok(None) => {
                        if verbose >= 2 {
                            log::info!("Server connection closed");
                        } else {
                            log::debug!("Server connection closed");
                        }
                        break;
                    }
                    Err(e) => {
                        log::error!("Error reading from server: {}", e);
                        break;
                    }
                }
            }
            
            if verbose >= 3 {
                log::debug!("Server reader thread exiting");
            }
        });

        self.protocol = Some(protocol);
        self.reader_thread = Some(reader_thread);

        if self.verbose >= 2 {
            log::info!("Successfully connected to dedups server");
        } else {
            log::debug!("Connected to dedups server");
        }
        
        Ok(())
    }

    /// Send a command to the server
    pub fn send_command(
        &mut self,
        command: String,
        args: Vec<String>,
        options: HashMap<String, String>,
    ) -> Result<()> {
        let protocol = self
            .protocol
            .as_mut()
            .ok_or_else(|| anyhow!("Not connected to server"))?;

        let cmd_msg = CommandMessage {
            command,
            args: args.clone(),
            options,
        };

        if self.verbose >= 3 {
            log::debug!("Sending command: {} {}", cmd_msg.command, args.join(" "));
        }

        let cmd_json = match serde_json::to_string(&cmd_msg) {
            Ok(json) => json,
            Err(e) => {
                if self.verbose >= 1 {
                    log::error!("Failed to serialize command: {}", e);
                }
                return Err(anyhow::anyhow!("Failed to serialize command: {}", e));
            }
        };
        
        let message = DedupMessage {
            message_type: MessageType::Command,
            payload: cmd_json,
        };

        match protocol.send_message(message) {
            Ok(_) => {
                if self.verbose >= 3 {
                    log::debug!("Command sent successfully");
                }
                Ok(())
            },
            Err(e) => {
                if self.verbose >= 1 {
                    log::error!("Failed to send command: {}", e);
                }
                Err(anyhow::anyhow!("Failed to send command: {}", e))
            }
        }
    }

    /// Receive the next message from the server
    pub fn receive_message(&self) -> Result<Option<DedupMessage>> {
        let receiver = self
            .message_receiver
            .as_ref()
            .ok_or_else(|| anyhow!("Not connected to server"))?;

        match receiver.recv_timeout(std::time::Duration::from_millis(100)) {
            Ok(msg) => {
                if self.verbose >= 3 {
                    log::debug!("Received message of type {:?}", msg.message_type);
                }
                Ok(Some(msg))
            },
            Err(mpsc::RecvTimeoutError::Timeout) => Ok(None),
            Err(mpsc::RecvTimeoutError::Disconnected) => {
                if self.verbose >= 1 {
                    log::error!("Server connection closed unexpectedly");
                }
                Err(anyhow!("Server connection closed"))
            },
        }
    }

    /// Disconnect from the server
    pub fn disconnect(&mut self) -> Result<()> {
        if self.verbose >= 3 {
            log::debug!("Disconnecting from server");
        }
        
        // Protocol will be dropped when self is dropped
        self.protocol = None;

        // Join the reader thread if it exists
        if let Some(thread) = self.reader_thread.take() {
            // We don't care about the result, just want to wait for it to finish
            let _ = thread.join();
            if self.verbose >= 3 {
                log::debug!("Reader thread terminated");
            }
        }

        self.message_receiver = None;

        if self.verbose >= 2 {
            log::info!("Disconnected from dedups server");
        } else {
            log::debug!("Disconnected from dedups server");
        }
        
        Ok(())
    }

    /// Execute a command and collect all output
    pub fn execute_command(
        &mut self,
        command: String,
        args: Vec<String>,
        options: HashMap<String, String>,
    ) -> Result<String> {
        if self.verbose >= 2 {
            log::info!("Executing command via API: {} {}", command, args.join(" "));
        }
        
        self.send_command(command, args, options)?;

        let mut output = String::new();
        let mut has_error = false;
        let mut timeout_count = 0;
        let max_timeouts = 50; // 5 seconds max wait time

        // Collect all messages until we get a Result or Error message
        loop {
            match self.receive_message()? {
                Some(msg) => {
                    match msg.message_type {
                        MessageType::Result => {
                            // Final result
                            if self.verbose >= 3 {
                                log::debug!("Received final result from server");
                            }
                            output.push_str(&msg.payload);
                            break;
                        }
                        MessageType::Error => {
                            // Error occurred
                            let error: ErrorMessage = match serde_json::from_str(&msg.payload) {
                                Ok(e) => e,
                                Err(parse_err) => {
                                    if self.verbose >= 1 {
                                        log::error!("Failed to parse error message: {}", parse_err);
                                        log::error!("Raw error payload: {}", msg.payload);
                                    }
                                    return Err(anyhow!("Failed to parse error message: {}", parse_err));
                                }
                            };
                            
                            if self.verbose >= 1 {
                                log::error!("Received error from server: {} (code {})", error.message, error.code);
                            }
                            
                            has_error = true;
                            output.push_str(&format!(
                                "Error (code {}): {}",
                                error.code, error.message
                            ));
                            break;
                        }
                        MessageType::Progress => {
                            // Progress update, add to output
                            let progress: ProgressMessage = match serde_json::from_str(&msg.payload) {
                                Ok(p) => p,
                                Err(e) => {
                                    if self.verbose >= 1 {
                                        log::warn!("Failed to parse progress message: {}", e);
                                    }
                                    continue;
                                }
                            };
                            
                            if self.verbose >= 2 {
                                log::info!(
                                    "Progress: {}% - {}",
                                    progress.percent_complete,
                                    progress.status_message
                                );
                            }
                            
                            output.push_str(&format!(
                                "Progress: {}% - {}\n",
                                progress.percent_complete, progress.status_message
                            ));
                            
                            // Reset timeout counter when we get progress
                            timeout_count = 0;
                        }
                        _ => {
                            // Forward raw JSON
                            if self.verbose >= 3 {
                                log::debug!("Received other message type: {:?}", msg.message_type);
                            }
                            output.push_str(&msg.payload);
                            output.push('\n');
                        }
                    }
                }
                None => {
                    // No message received in timeout period, continue waiting
                    timeout_count += 1;
                    
                    if timeout_count >= max_timeouts {
                        if self.verbose >= 1 {
                            log::warn!("Timed out waiting for server response after {} attempts", max_timeouts);
                        }
                        break;
                    }
                    
                    thread::sleep(std::time::Duration::from_millis(100));
                }
            }
        }

        if self.verbose >= 3 {
            log::debug!("Command execution completed");
        }

        if has_error {
            Err(anyhow!(output))
        } else {
            Ok(output)
        }
    }
}
