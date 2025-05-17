#[cfg(feature = "ssh")]
use crate::protocol::{
    CommandMessage, DedupMessage, ErrorMessage, MessageType, ProgressMessage, ProtocolHandler,
    ResultMessage, TcpProtocolHandler,
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
use std::sync::mpsc::{self, Receiver, Sender};
#[cfg(feature = "ssh")]
use std::thread::{self, JoinHandle};

/// Client for communicating with a dedups server
#[cfg(feature = "ssh")]
pub struct DedupClient {
    host: String,
    port: u16,
    protocol: Option<TcpProtocolHandler>,
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

        let protocol = TcpProtocolHandler::new(stream)?;

        // Create a message channel for receiving messages from the server
        let (tx, rx) = mpsc::channel();
        self.message_receiver = Some(rx);

        // Start a background thread to read messages from the server
        let mut reader_protocol = protocol
            .stream
            .try_clone()
            .context("Failed to clone stream for reader thread")?;
        let reader_thread = thread::spawn(move || {
            let mut buf = Vec::new();
            let mut buf_reader = std::io::BufReader::new(&mut reader_protocol);

            loop {
                buf.clear();
                let bytes_read =
                    match std::io::BufRead::read_until(&mut buf_reader, b'\n', &mut buf) {
                        Ok(0) => break, // EOF
                        Ok(n) => n,
                        Err(e) => {
                            log::error!("Error reading from server: {}", e);
                            break;
                        }
                    };

                if bytes_read > 0 {
                    let line = match String::from_utf8(buf.clone()) {
                        Ok(s) => s,
                        Err(e) => {
                            log::error!("Invalid UTF-8 in server response: {}", e);
                            continue;
                        }
                    };

                    log::debug!("Received raw message: {}", line);

                    // Try to parse as DedupMessage
                    if line.starts_with('{') && (line.trim_end().ends_with('}')) {
                        match serde_json::from_str::<DedupMessage>(&line) {
                            Ok(msg) => {
                                if tx.send(msg).is_err() {
                                    log::error!(
                                        "Failed to send message to channel, receiver dropped"
                                    );
                                    break;
                                }
                            }
                            Err(e) => {
                                // Try as raw JSON to be forwarded to caller
                                log::debug!("Not a DedupMessage, forwarding raw JSON: {}", e);
                                let raw_msg = DedupMessage {
                                    message_type: MessageType::Result,
                                    payload: line.clone(),
                                };
                                if tx.send(raw_msg).is_err() {
                                    log::error!("Failed to send raw message to channel");
                                    break;
                                }
                            }
                        }
                    } else {
                        log::debug!("Ignoring non-JSON line: {}", line);
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
