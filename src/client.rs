#[cfg(feature = "ssh")]
use crate::options::DedupOptions;
#[cfg(feature = "ssh")]
use crate::protocol::{
    create_protocol_handler, CommandMessage, DedupMessage, ErrorMessage, MessageType,
    ProgressMessage, ProtocolHandler,
};
#[cfg(feature = "ssh")]
use anyhow::{anyhow, Context, Result};
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
        }
    }

    pub fn with_options(host: String, port: u16, options: DedupOptions) -> Self {
        Self {
            host,
            port,
            protocol: None,
            options,
            message_receiver: None,
            reader_thread: None,
        }
    }

    /// Connect to the dedups server
    pub fn connect(&mut self) -> Result<()> {
        log::info!("Connecting to dedups server at {}:{}", self.host, self.port);

        let stream =
            TcpStream::connect(format!("{}:{}", self.host, self.port)).with_context(|| {
                format!(
                    "Failed to connect to dedups server at {}:{}",
                    self.host, self.port
                )
            })?;

        // Create protocol handler based on options
        let use_protobuf = {
            #[cfg(feature = "proto")]
            {
                self.options.use_protobuf
            }
            #[cfg(not(feature = "proto"))]
            {
                false
            }
        };

        let use_compression = {
            #[cfg(feature = "proto")]
            {
                self.options.use_compression
            }
            #[cfg(not(feature = "proto"))]
            {
                false
            }
        };

        let compression_level = {
            #[cfg(feature = "proto")]
            {
                self.options.compression_level
            }
            #[cfg(not(feature = "proto"))]
            {
                3
            }
        };

        let protocol = create_protocol_handler(
            stream.try_clone()?,
            use_protobuf,
            use_compression,
            compression_level,
        )?;

        // Create a message channel for receiving messages from the server
        let (tx, rx) = mpsc::channel();
        self.message_receiver = Some(rx);

        // Start a background thread to read messages from the server
        let mut reader_protocol =
            create_protocol_handler(stream, use_protobuf, use_compression, compression_level)?;

        let reader_thread = thread::spawn(move || {
            loop {
                match reader_protocol.receive_message() {
                    Ok(Some(msg)) => {
                        if tx.send(msg).is_err() {
                            log::error!("Failed to send message to channel, receiver dropped");
                            break;
                        }
                    }
                    Ok(None) => {
                        log::info!("Server connection closed");
                        break;
                    }
                    Err(e) => {
                        log::error!("Error reading from server: {}", e);
                        break;
                    }
                }
            }
            log::info!("Server reader thread exiting");
        });

        self.protocol = Some(protocol);
        self.reader_thread = Some(reader_thread);

        log::info!("Connected to dedups server");
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
            args,
            options,
        };

        let cmd_json = serde_json::to_string(&cmd_msg)?;
        let message = DedupMessage {
            message_type: MessageType::Command,
            payload: cmd_json,
        };

        protocol.send_message(message)?;
        Ok(())
    }

    /// Receive the next message from the server
    pub fn receive_message(&self) -> Result<Option<DedupMessage>> {
        let receiver = self
            .message_receiver
            .as_ref()
            .ok_or_else(|| anyhow!("Not connected to server"))?;

        match receiver.recv_timeout(std::time::Duration::from_millis(100)) {
            Ok(msg) => Ok(Some(msg)),
            Err(mpsc::RecvTimeoutError::Timeout) => Ok(None),
            Err(mpsc::RecvTimeoutError::Disconnected) => Err(anyhow!("Server connection closed")),
        }
    }

    /// Disconnect from the server
    pub fn disconnect(&mut self) -> Result<()> {
        // Protocol will be dropped when self is dropped
        self.protocol = None;

        // Join the reader thread if it exists
        if let Some(thread) = self.reader_thread.take() {
            // We don't care about the result, just want to wait for it to finish
            let _ = thread.join();
        }

        self.message_receiver = None;

        log::info!("Disconnected from dedups server");
        Ok(())
    }

    /// Execute a command and collect all output
    pub fn execute_command(
        &mut self,
        command: String,
        args: Vec<String>,
        options: HashMap<String, String>,
    ) -> Result<String> {
        self.send_command(command, args, options)?;

        let mut output = String::new();
        let mut has_error = false;

        // Collect all messages until we get a Result or Error message
        loop {
            match self.receive_message()? {
                Some(msg) => {
                    match msg.message_type {
                        MessageType::Result => {
                            // Final result
                            output.push_str(&msg.payload);
                            break;
                        }
                        MessageType::Error => {
                            // Error occurred
                            let error: ErrorMessage = serde_json::from_str(&msg.payload)?;
                            has_error = true;
                            output.push_str(&format!(
                                "Error (code {}): {}",
                                error.code, error.message
                            ));
                            break;
                        }
                        MessageType::Progress => {
                            // Progress update, add to output
                            let progress: ProgressMessage = serde_json::from_str(&msg.payload)?;
                            output.push_str(&format!(
                                "Progress: {}% - {}\n",
                                progress.percent_complete, progress.status_message
                            ));
                        }
                        _ => {
                            // Forward raw JSON
                            output.push_str(&msg.payload);
                            output.push('\n');
                        }
                    }
                }
                None => {
                    // No message received in timeout period, continue waiting
                    thread::sleep(std::time::Duration::from_millis(10));
                }
            }
        }

        if has_error {
            Err(anyhow!(output))
        } else {
            Ok(output)
        }
    }
}
