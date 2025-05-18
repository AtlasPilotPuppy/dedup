#[cfg(feature = "ssh")]
use anyhow::{Context, Result};
#[cfg(feature = "ssh")]
use ssh2::Session;
use std::path::{Path, PathBuf};

/// Represents a remote location parsed from an SSH URI
#[cfg(feature = "ssh")]
#[derive(Debug, Clone)]
pub struct RemoteLocation {
    pub username: Option<String>,
    pub host: String,
    pub port: Option<u16>,
    pub path: PathBuf,
    pub ssh_options: Vec<String>,
    pub rsync_options: Vec<String>,
}

#[cfg(feature = "ssh")]
impl RemoteLocation {
    /// Parses SSH format strings like:
    /// - `ssh:host:/path`
    /// - `ssh:user@host:/path`
    /// - `ssh:user@host:port:/path`
    /// - `ssh:host:/path:ssh_opt1,ssh_opt2:rsync_opt1,rsync_opt2`
    pub fn parse(location_str: &str) -> Result<Self> {
        // Check if it starts with ssh:
        if !location_str.starts_with("ssh:") {
            return Err(anyhow::anyhow!("Not an SSH location: {}", location_str));
        }

        // Remove the ssh: prefix
        let without_prefix = &location_str[4..];

        // Split the path by colon to handle the various parts
        let parts: Vec<&str> = without_prefix.splitn(5, ':').collect();

        // Need at least host and path
        if parts.len() < 2 {
            return Err(anyhow::anyhow!(
                "Invalid SSH format. Expected at least ssh:host:/path, got: {}",
                location_str
            ));
        }

        // Parse host and optional user
        let host_part = parts[0];
        let (username, host) = if host_part.contains('@') {
            let host_parts: Vec<&str> = host_part.split('@').collect();
            (Some(host_parts[0].to_string()), host_parts[1].to_string())
        } else {
            (None, host_part.to_string())
        };

        // Determine the position of path and optional port
        let (port, path_idx) = if parts.len() >= 3 && parts[1].parse::<u16>().is_ok() {
            (Some(parts[1].parse::<u16>()?), 2)
        } else {
            (None, 1)
        };

        // Get the path
        let path = PathBuf::from(parts[path_idx]);

        // Get optional SSH and Rsync options
        let mut ssh_options = Vec::new();
        let mut rsync_options = Vec::new();

        if parts.len() > path_idx + 1 && !parts[path_idx + 1].is_empty() {
            ssh_options = parts[path_idx + 1]
                .split(',')
                .map(|s| s.to_string())
                .collect();
        }

        if parts.len() > path_idx + 2 && !parts[path_idx + 2].is_empty() {
            rsync_options = parts[path_idx + 2]
                .split(',')
                .map(|s| s.to_string())
                .collect();
        }

        Ok(RemoteLocation {
            username,
            host,
            port,
            path,
            ssh_options,
            rsync_options,
        })
    }

    /// Check if a path is a remote SSH path
    pub fn is_ssh_path(path: &str) -> bool {
        path.starts_with("ssh:")
    }

    /// Generate SSH command with options
    pub fn ssh_command(&self) -> Vec<String> {
        let mut cmd = vec!["ssh".to_string()];

        // Add port if specified
        if let Some(port) = self.port {
            cmd.push("-p".to_string());
            cmd.push(port.to_string());
        }

        // Add custom SSH options
        cmd.extend(self.ssh_options.clone());

        // Add host with optional user
        let host = if let Some(user) = &self.username {
            format!("{}@{}", user, self.host)
        } else {
            self.host.clone()
        };

        cmd.push(host);
        cmd
    }

    /// Generate Rsync command with options for copying files
    pub fn rsync_command(&self, source: &Path, dest: &Path, is_remote_source: bool) -> Vec<String> {
        let mut cmd = vec!["rsync".to_string(), "-avz".to_string()];

        // Add port if specified
        if let Some(port) = self.port {
            cmd.push("-e".to_string());
            cmd.push(format!("ssh -p {}", port));
        } else if !self.ssh_options.is_empty() {
            // Add custom SSH options if available
            let ssh_opts = self.ssh_options.join(" ");
            cmd.push("-e".to_string());
            cmd.push(format!("ssh {}", ssh_opts));
        }

        // Add custom Rsync options
        cmd.extend(self.rsync_options.clone());

        // Format source and destination with proper remote syntax
        // Don't include port in host_part if we're already specifying it with -e
        let host_part = if let Some(user) = &self.username {
            format!("{}@{}", user, self.host)
        } else {
            self.host.clone()
        };

        let source_str = if is_remote_source {
            format!("{}:{}", host_part, source.display())
        } else {
            source.display().to_string()
        };

        let dest_str = if !is_remote_source {
            format!("{}:{}", host_part, dest.display())
        } else {
            dest.display().to_string()
        };

        cmd.push(source_str);
        cmd.push(dest_str);

        cmd
    }

    /// Run a command on the remote system
    pub fn run_command(&self, command: &str) -> Result<String> {
        // Check if SSH_COMMAND environment variable is set to use custom SSH command
        if let Ok(ssh_command) = std::env::var("SSH_COMMAND") {
            log::debug!("Using SSH command from environment: {}", ssh_command);

            // Build full command with custom SSH command
            let host_part = if let Some(user) = &self.username {
                if let Some(port) = self.port {
                    format!("{}@{}:{}", user, self.host, port)
                } else {
                    format!("{}@{}", user, self.host)
                }
            } else if let Some(port) = self.port {
                format!("{}:{}", self.host, port)
            } else {
                self.host.clone()
            };

            // Execute with custom SSH wrapper
            let output = std::process::Command::new(&ssh_command)
                .arg(&host_part)
                .arg(command)
                .output()
                .with_context(|| format!("Failed to execute SSH command using wrapper. Please verify SSH access to host '{}' is configured correctly.", self.host))?;

            let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
            let stderr = String::from_utf8_lossy(&output.stderr).into_owned();

            log::debug!("SSH command stdout: {}", stdout);
            log::debug!("SSH command stderr: {}", stderr);

            if !output.status.success() {
                log::error!("SSH command failed. Stderr: {}", stderr);
                log::error!("SSH command stdout: {}", stdout);
                return Err(anyhow::anyhow!(
                    "SSH command failed on host '{}': {}",
                    self.host,
                    stderr
                ));
            }

            Ok(stdout)
        } else {
            // Standard SSH approach
            let mut ssh_cmd = vec!["ssh".to_string()];

            // Add port if specified
            if let Some(port) = self.port {
                ssh_cmd.extend(vec!["-p".to_string(), port.to_string()]);
            }

            // Add SSH config file if specified in environment
            if let Ok(config_file) = std::env::var("SSH_CONFIG_FILE") {
                if !config_file.is_empty() {
                    log::debug!("Using SSH config file from environment: {}", config_file);
                    ssh_cmd.extend(vec!["-F".to_string(), config_file]);
                }
            } else {
                // Always use system SSH config by default
                ssh_cmd.extend(vec![
                    "-F".to_string(),
                    shellexpand::tilde("~/.ssh/config").into_owned(),
                ]);
            }

            // Add any custom SSH options
            for opt in &self.ssh_options {
                // Check if it's a key=value pair
                if opt.contains('=') {
                    let parts: Vec<&str> = opt.split('=').collect();
                    if parts.len() == 2 {
                        let key = parts[0];
                        let value = parts[1];

                        // Special handling for common options
                        match key {
                            "IdentityFile" => {
                                ssh_cmd.push("-i".to_string());
                                ssh_cmd.push(value.to_string());
                            }
                            _ => {
                                // Generic option
                                ssh_cmd.push("-o".to_string());
                                ssh_cmd.push(format!("{}={}", key, value));
                            }
                        }
                    } else {
                        // Not a proper key=value, add as-is
                        ssh_cmd.push(opt.clone());
                    }
                } else {
                    // Not a key=value, add as-is
                    ssh_cmd.push(opt.clone());
                }
            }

            // Add host with optional user
            let host = if let Some(user) = &self.username {
                format!("{}@{}", user, self.host)
            } else {
                self.host.clone()
            };
            ssh_cmd.push(host);

            // Add the command
            ssh_cmd.push(command.to_string());

            // Log the command being executed
            log::debug!("Executing SSH command: {}", ssh_cmd.join(" "));

            // Execute the command
            let output = std::process::Command::new(&ssh_cmd[0])
                .args(&ssh_cmd[1..])
                .output()
                .with_context(|| format!("Failed to execute SSH command. Please verify SSH access to host '{}' is configured correctly.", self.host))?;

            let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
            let stderr = String::from_utf8_lossy(&output.stderr).into_owned();

            log::debug!("SSH command stdout: {}", stdout);
            log::debug!("SSH command stderr: {}", stderr);

            if !output.status.success() {
                log::error!("SSH command failed. Stderr: {}", stderr);
                log::error!("SSH command stdout: {}", stdout);
                return Err(anyhow::anyhow!(
                    "SSH command failed on host '{}': {}",
                    self.host,
                    stderr
                ));
            }

            Ok(stdout)
        }
    }

    /// Check if dedups is installed on the remote system
    pub async fn check_dedups_installed(&self) -> Result<Option<String>> {
        log::info!(
            "Checking if dedups is installed on remote host '{}'...",
            self.host
        );

        // Set up environment and check for dedups
        let check_cmd = r#"
            export PATH="$HOME/.local/bin:$PATH"
            if [ -f "$HOME/.bashrc" ]; then
                source "$HOME/.bashrc"
            fi
            if [ -f "$HOME/.profile" ]; then
                source "$HOME/.profile"
            fi
            which dedups 2>/dev/null || echo 'not found'
        "#;

        match self.run_command(check_cmd) {
            Ok(output) => {
                let output_trim = output.trim();
                // Sometimes 'which' may output messages before 'not found'. Check if any line contains dedups path.
                if !output_trim.is_empty() && !output_trim.contains("not found") {
                    log::info!(
                        "Found dedups on remote host '{}' at: {}",
                        self.host,
                        output_trim
                    );
                    // Take first line as path
                    let first_line = output_trim.lines().next().unwrap_or("").to_string();
                    Ok(Some(first_line))
                } else {
                    log::info!("dedups not found on remote host '{}'", self.host);
                    Ok(None)
                }
            }
            Err(e) => {
                log::warn!("Failed to check for dedups on remote host: {}", e);
                Ok(None)
            }
        }
    }

    /// Install dedups on the remote system
    pub fn install_dedups(&self, cli: &crate::Cli) -> Result<String> {
        log::info!(
            "Attempting to install dedups on remote host '{}'...",
            self.host
        );

        // First check if we want to use sudo and if it's available
        let has_sudo = if cli.use_sudo {
            match self.run_command("sudo -n true 2>/dev/null && echo 'yes' || echo 'no'") {
                Ok(result) => result.trim() == "yes",
                Err(_) => {
                    log::info!("Sudo requires password, will prompt during installation");
                    true // We'll try with sudo anyway since we're allowed to prompt
                }
            }
        } else {
            false
        };

        let install_dir = if has_sudo {
            log::info!("Will install to /usr/local/bin using sudo");
            "/usr/local/bin"
        } else {
            log::info!("Will install to user's ~/.local/bin");
            "~/.local/bin"
        };

        // Create install directory if it doesn't exist
        let mkdir_cmd = if has_sudo {
            format!(
                "sudo mkdir -p {} && sudo chown $USER {}",
                install_dir, install_dir
            )
        } else {
            format!("mkdir -p {}", install_dir)
        };
        log::debug!("Creating installation directory: {}", install_dir);
        self.run_command(&mkdir_cmd)?;

        // Set up PATH for both current session and future sessions
        let path_setup = r#"
            if [[ ":$PATH:" != *":$HOME/.local/bin:"* ]]; then
                export PATH="$HOME/.local/bin:$PATH"
                echo 'export PATH="$HOME/.local/bin:$PATH"' >> ~/.bashrc
                echo 'export PATH="$HOME/.local/bin:$PATH"' >> ~/.zshrc 2>/dev/null || true
                echo 'export PATH="$HOME/.local/bin:$PATH"' >> ~/.profile
            fi
        "#;

        if let Err(e) = self.run_command(path_setup) {
            log::warn!("Failed to update PATH in shell config: {}", e);
        }

        // Download and install dedups
        let install_cmd = format!(
            "curl -sSL https://raw.githubusercontent.com/AtlasPilotPuppy/dedup/main/install.sh | {} bash -s -- --ssh",
            if has_sudo { "sudo -S" } else { "" }
        );

        log::info!("Downloading and installing dedups on remote host...");
        match self.run_command(&install_cmd) {
            Ok(_) => {
                // Verify installation with updated PATH
                let verify_cmd = r#"
                    export PATH="$HOME/.local/bin:$PATH"
                    if [ -f "$HOME/.bashrc" ]; then
                        source "$HOME/.bashrc"
                    fi
                    if [ -f "$HOME/.profile" ]; then
                        source "$HOME/.profile"
                    fi
                    which dedups || echo 'not found'
                "#;

                match self.run_command(verify_cmd) {
                    Ok(path) => {
                        let path = path.trim();
                        if path != "not found" {
                            log::info!(
                                "Successfully installed dedups on remote host '{}' at: {}",
                                self.host,
                                path
                            );
                            Ok(path.to_string())
                        } else {
                            log::error!(
                                "Installation appeared to succeed but dedups not found in PATH"
                            );
                            Err(anyhow::anyhow!(
                                "Installation verification failed: dedups not found in PATH"
                            ))
                        }
                    }
                    Err(e) => {
                        log::error!("Failed to verify installation: {}", e);
                        Err(anyhow::anyhow!("Installation verification failed: {}", e))
                    }
                }
            }
            Err(e) => {
                log::error!(
                    "Failed to install dedups on remote host '{}': {}",
                    self.host,
                    e
                );
                Err(anyhow::anyhow!(
                    "Failed to install dedups on remote host '{}': {}",
                    self.host,
                    e
                ))
            }
        }
    }

    /// Check if a remote directory exists
    pub fn check_directory_exists(&self) -> Result<bool> {
        log::info!(
            "Checking if remote directory exists: {}",
            self.path.display()
        );

        // First try to establish basic SSH connectivity
        let test_cmd = "echo 'SSH connection test successful'";
        match self.run_command(test_cmd) {
            Ok(_) => log::info!("SSH connection test successful"),
            Err(e) => {
                log::error!("SSH connection test failed: {}", e);
                return Err(anyhow::anyhow!(
                    "Failed to establish SSH connection to host '{}'. Please verify:\n\
                    1. The host '{}' is configured in ~/.ssh/config\n\
                    2. You have SSH key access to the host\n\
                    3. The host is reachable\n\
                    Error: {}",
                    self.host,
                    self.host,
                    e
                ));
            }
        }

        // Then check if the directory exists
        let check_cmd = format!(
            "test -d '{}' && echo 'EXISTS' || echo 'NOTFOUND'",
            self.path.display()
        );
        match self.run_command(&check_cmd) {
            Ok(output) => {
                let exists = output.trim() == "EXISTS";
                log::info!(
                    "Remote directory {} {} on host '{}'",
                    self.path.display(),
                    if exists { "exists" } else { "does not exist" },
                    self.host
                );
                Ok(exists)
            }
            Err(e) => {
                log::error!("Failed to check remote directory: {}", e);
                Err(e)
            }
        }
    }
}

/// SSH Communication protocol for dedups instances
#[cfg(feature = "ssh")]
pub struct SshProtocol {
    session: Option<Session>,
    remote: RemoteLocation,
}

#[cfg(feature = "ssh")]
impl std::fmt::Debug for SshProtocol {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SshProtocol")
            .field("remote", &self.remote)
            .field("session", &(self.session.is_some()))
            .finish()
    }
}

#[cfg(feature = "ssh")]
impl Clone for SshProtocol {
    fn clone(&self) -> Self {
        // We can't clone the session, so create a new one without a session
        SshProtocol {
            session: None,
            remote: self.remote.clone(),
        }
    }
}

#[cfg(feature = "ssh")]
impl SshProtocol {
    /// Create a new SSH protocol handler
    pub fn new(remote: RemoteLocation) -> Self {
        SshProtocol {
            session: None,
            remote,
        }
    }

    /// Connect to the remote host
    pub fn connect(&mut self) -> Result<()> {
        log::debug!(
            "Starting SSH connection to host '{}' with options: {:?}",
            self.remote.host,
            self.remote.ssh_options
        );

        // Build SSH command
        let mut ssh_cmd = vec!["ssh".to_string()];

        // Add port if specified
        if let Some(port) = self.remote.port {
            log::debug!("Using custom port: {}", port);
            ssh_cmd.extend(vec!["-p".to_string(), port.to_string()]);
        }

        // Add custom SSH options
        if !self.remote.ssh_options.is_empty() {
            log::debug!("Adding custom SSH options: {:?}", self.remote.ssh_options);
            ssh_cmd.extend(self.remote.ssh_options.clone());
        }

        // Add host with optional user
        let host = if let Some(user) = &self.remote.username {
            log::debug!("Using username: {}", user);
            format!("{}@{}", user, self.remote.host)
        } else {
            self.remote.host.clone()
        };
        ssh_cmd.push(host);

        // Test SSH connection first
        let test_cmd = format!("{} echo 'SSH connection test'", ssh_cmd.join(" "));
        log::debug!("Testing SSH connection with command: {}", test_cmd);

        let output = std::process::Command::new("sh")
            .arg("-c")
            .arg(&test_cmd)
            .output()
            .with_context(|| {
                format!(
                    "Failed to execute SSH test command for host '{}'",
                    self.remote.host
                )
            })?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            let stdout = String::from_utf8_lossy(&output.stdout);
            log::error!("SSH test command failed. Stderr: {}", stderr);
            log::error!("SSH test command stdout: {}", stdout);
            return Err(anyhow::anyhow!("SSH connection test failed: {}", stderr));
        }

        log::debug!("SSH connection test successful");

        // Create new SSH session for future use
        let sess = Session::new()?;
        self.session = Some(sess);

        Ok(())
    }

    /// Helper: extract JSON lines from mixed output
    fn extract_json_lines(text: &str) -> Option<String> {
        let mut json_lines = Vec::new();
        for line in text.lines() {
            let trimmed = line.trim_start_matches('\u{1e}').trim(); // strip RS char if present
            if trimmed.starts_with('{') && trimmed.ends_with('}') {
                json_lines.push(trimmed);
            }
        }
        if json_lines.is_empty() {
            None
        } else {
            Some(json_lines.join("\n"))
        }
    }

    /// Execute dedups command on remote system with support for streaming JSON output
    /// Uses a dedicated socket/tunnel approach for better streaming
    pub fn execute_dedups(&self, args: &[&str], cli: &crate::Cli) -> Result<String> {
        // Check for protocol conflicts
        if cli.json && cli.use_protobuf {
            return Err(anyhow::anyhow!(
                "Cannot use both JSON and Protobuf protocols. Protobuf is preferred for tunnel mode."
            ));
        }

        // First check if we should use tunnel mode
        let should_use_tunnel = cli.use_ssh_tunnel && cli.tunnel_api_mode;

        if should_use_tunnel {
            // Check verbosity level for additional logs
            if cli.verbose >= 2 {
                log::info!("Attempting tunnel-based API communication");
            }

            // First verify remote dedups is available (do this ONCE)
            let remote_dedups = tokio::runtime::Runtime::new()
                .unwrap()
                .block_on(self.remote.check_dedups_installed())?;

            match remote_dedups {
                Some(path) => {
                    log::info!("Found remote dedups at {}, starting server mode", path);
                    match self.execute_dedups_with_tunnel(args, cli, &path) {
                        Ok(result) => Ok(result),
                        Err(e) => {
                            log::error!("Tunnel mode failed: {}", e);
                            if cli.media_mode {
                                // Media mode requires tunnel mode
                                return Err(anyhow::anyhow!(
                                    "Media mode requires working tunnel mode communication. Error: {}", e
                                ));
                            }
                            log::warn!("Falling back to standard SSH mode with prefixed JSON");
                            self.execute_dedups_standard(args, cli)
                        }
                    }
                }
                None => {
                    if cli.media_mode {
                        return Err(anyhow::anyhow!(
                            "Media mode requires dedups to be installed on the remote system"
                        ));
                    }
                    log::warn!("Remote dedups not found, using standard SSH mode");
                    self.execute_dedups_standard(args, cli)
                }
            }
        } else {
            // Standard mode requested
            log::info!("Using standard SSH mode (JSON with prefix)");
            self.execute_dedups_standard(args, cli)
        }
    }

    /// Execute dedups via an SSH tunnel for API communication
    pub fn execute_dedups_with_tunnel(&self, args: &[&str], cli: &crate::Cli, remote_dedups_path: &str) -> Result<String> {
        use crate::client::DedupClient;
        use crate::protocol::find_available_port;
        use std::collections::HashMap;

        if cli.verbose >= 2 {
            log::info!("Setting up tunnel-based API communication");
        }

        // Find an available port for the tunnel
        let local_port = if cli.port > 0 {
            cli.port
        } else {
            find_available_port(10000, 11000)
                .context("Could not find an available port for SSH tunnel")?
        };

        if cli.verbose >= 2 {
            log::info!("Using port {} for API communication", local_port);
        }

        // Set up the SSH tunnel options
        let port_forwarding = format!("{}:localhost:{}", local_port, local_port);
        let ssh_options = self.get_ssh_options(cli);
        let mut ssh_opts = vec![
            "-N".to_string(),  // No command execution, just forwarding
            "-L".to_string(), port_forwarding,
            "-o".to_string(), "ExitOnForwardFailure=yes".to_string()  // Fail if port forwarding fails
        ];
        ssh_opts.extend(ssh_options);

        let remote_addr = if let Some(user) = &self.remote.username {
            if let Some(port) = self.remote.port {
                format!("{}@{}:{}", user, self.remote.host, port)
            } else {
                format!("{}@{}", user, self.remote.host)
            }
        } else {
            self.remote.host.clone()
        };

        // Start the tunnel in the background
        let tunnel_cmd = format!(
            "ssh {} {} -f",  // -f backgrounds the tunnel
            ssh_opts.join(" "),
            remote_addr
        );

        if cli.verbose >= 2 {
            log::info!("Starting SSH tunnel: {}", tunnel_cmd);
        }

        // Start the tunnel
        let tunnel_status = std::process::Command::new("sh")
            .arg("-c")
            .arg(&tunnel_cmd)
            .status()
            .context("Failed to start SSH tunnel")?;

        if !tunnel_status.success() {
            return Err(anyhow::anyhow!("Failed to establish SSH tunnel"));
        }

        // Now start the remote server
        let port_str = local_port.to_string();
        let mut server_flags = vec![
            "--server-mode".to_string(),
            "--port".to_string(), port_str,
            "--use-protobuf".to_string(),  // Always use protobuf in tunnel mode
            "--use-compression".to_string()
        ];

        // Add verbosity flags
        if cli.verbose > 0 {
            for _ in 0..cli.verbose {
                server_flags.push("-v".to_string());
            }
        }

        // If we have a directory, pass it
        if !cli.directories.is_empty() {
            let dir_str = cli.directories[0].to_string_lossy().into_owned();
            server_flags.push(dir_str);
        }

        // Construct the server startup command
        let server_cmd = format!(
            r#"ssh {} {} "export PATH={}:$PATH; {} {} {}""#,
            self.get_ssh_options(cli).join(" "),
            remote_addr,
            std::path::Path::new(remote_dedups_path).parent().unwrap_or(std::path::Path::new("/usr/local/bin")).display(),
            if let Ok(val) = std::env::var("RUST_LOG") {
                format!("export RUST_LOG={};", val)
            } else {
                String::new()
            },
            remote_dedups_path,
            server_flags.join(" ")
        );

        if cli.verbose >= 2 {
            log::info!("Starting remote server: {}", server_cmd);
        }

        // Start the server
        let _server_child = std::process::Command::new("sh")
            .arg("-c")
            .arg(&server_cmd)
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()
            .context("Failed to start remote server")?;

        // Wait a moment for the server to start
        std::thread::sleep(std::time::Duration::from_secs(1));

        // Create the client
        let mut client = DedupClient::with_options(
            "localhost".to_string(),
            local_port,
            crate::options::DedupOptions {
                use_ssh_tunnel: true,
                tunnel_api_mode: true,
                port: local_port,
                use_protobuf: true,
                use_compression: true,
                ..Default::default()
            }
        );

        // Try to connect with retries
        let mut connected = false;
        for attempt in 1..=5 {
            match client.connect() {
                Ok(_) => {
                    connected = true;
                    break;
                }
                Err(e) => {
                    if attempt == 5 {
                        log::error!("Failed to connect to server: {}", e);
                        return Err(e);
                    }
                    std::thread::sleep(std::time::Duration::from_millis(500));
                }
            }
        }

        if !connected {
            return Err(anyhow::anyhow!("Failed to connect to remote server"));
        }

        // Execute the command
        let result = client.execute_command(
            "dedups".to_string(),
            args.iter().map(|s| s.to_string()).collect(),
            HashMap::new()
        )?;

        Ok(result)
    }

    /// Standard execution method (fallback) - uses JSON with prefix for reliable parsing
    fn execute_dedups_standard(&self, args: &[&str], cli: &crate::Cli) -> Result<String> {
        // Add JSON prefix to help with parsing
        let mut modified_args = args.to_vec();
        modified_args.push("--json-prefix");  // This needs to be added to the CLI options

        // Build SSH command
        let mut ssh_cmd = vec!["ssh".to_string()];

        // Add port if specified
        if let Some(port) = self.remote.port {
            ssh_cmd.extend(vec!["-p".to_string(), port.to_string()]);
        }

        // Add custom SSH options
        ssh_cmd.extend(self.remote.ssh_options.clone());

        // Check if we're using JSON output
        let using_json = args.contains(&"--json");

        // For JSON output, add options to minimize SSH noise
        if using_json {
            // Add options to prevent MOTD, banner, etc.
            ssh_cmd.extend(vec![
                "-T".to_string(), // Disable pseudo-terminal allocation
                "-o".to_string(),
                "LogLevel=QUIET".to_string(),
                "-o".to_string(),
                "UserKnownHostsFile=/dev/null".to_string(),
                "-o".to_string(),
                "StrictHostKeyChecking=no".to_string(),
                "-o".to_string(),
                "BatchMode=yes".to_string(),
            ]);
        }

        // Add host with optional user
        let host = if let Some(user) = &self.remote.username {
            format!("{}@{}", user, self.remote.host)
        } else {
            self.remote.host.clone()
        };
        ssh_cmd.push(host);

        // Set up environment and command
        let rust_log_export = std::env::var("RUST_LOG").ok();
        let setup_env = format!(
            "export PATH=\"$HOME/.local/bin:$PATH\"; {} if [ -f \"$HOME/.bashrc\" ]; then source \"$HOME/.bashrc\"; fi; if [ -f \"$HOME/.profile\" ]; then source \"$HOME/.profile\"; fi;",
            rust_log_export
                .map(|v| format!("export RUST_LOG=\"{}\";", v))
                .unwrap_or_default()
        );

        // Determine explicit dedups binary path
        let remote_dedups_bin: String = {
            match tokio::runtime::Runtime::new()
                .unwrap()
                .block_on(self.remote.check_dedups_installed())
            {
                Ok(Some(path)) if !path.trim().is_empty() => path.trim().to_string(),
                _ => "dedups".to_string(),
            }
        };

        // Build the command, adding unbuffer for JSON mode
        let command = if using_json {
            format!(
                "{}\n                # Try to use stdbuf/unbuffer to reduce buffering issues with JSON output\n                if command -v stdbuf >/dev/null 2>&1; then\n                    stdbuf -o0 -e0 {dedups_bin} {}\n                elif command -v unbuffer >/dev/null 2>&1; then\n                    unbuffer {dedups_bin} {}\n                else\n                    # No unbuffering tools available, proceed normally but might have buffering issues\n                    {dedups_bin} {}\n                fi",
                setup_env,
                args.join(" "),
                args.join(" "),
                args.join(" "),
                dedups_bin = remote_dedups_bin
            )
        } else {
            format!("{}\n{dedups_bin} {}", setup_env, args.join(" "), dedups_bin = remote_dedups_bin)
        };

        ssh_cmd.push(command);

        log::debug!("Executing remote command: {}", ssh_cmd.join(" "));

        // Use different approach for JSON to handle streaming
        if using_json {
            let mut command = std::process::Command::new(&ssh_cmd[0]);
            command.args(&ssh_cmd[1..]);

            // Create pipes for stdout and stderr
            let mut child = command
                .stdout(std::process::Stdio::piped())
                .stderr(std::process::Stdio::piped())
                .spawn()
                .with_context(|| {
                    format!("Failed to execute command on host '{}'", self.remote.host)
                })?;

            // Create a reader for stdout
            let stdout = child.stdout.take().unwrap();
            let mut reader = std::io::BufReader::new(stdout);
            let mut line = String::new();
            let mut output = String::new();

            // Process each line as it comes in
            use std::io::BufRead;

            while let Ok(bytes) = reader.read_line(&mut line) {
                if bytes == 0 {
                    break; // End of stream
                }

                // If this is valid JSON, pass it through immediately
                if line.trim().starts_with('{') {
                    // Pass through the JSON line to stdout
                    println!("{}", line.trim());
                }

                // Accumulate the full output
                output.push_str(&line);
                line.clear();
            }

            // Check if the command succeeded
            let status = child.wait().with_context(|| {
                format!("Failed to wait for command on host '{}'", self.remote.host)
            })?;

            if !status.success() {
                // Check if the output contains any valid JSON
                if output.contains("\"type\":\"error\"") {
                    // Error was already output in JSON format, just return it
                    Ok(output)
                } else {
                    // Get stderr
                    let mut stderr = String::new();
                    if let Some(mut stderr_handle) = child.stderr {
                        use std::io::Read;
                        let _ = stderr_handle.read_to_string(&mut stderr);
                    }

                    if !stderr.is_empty() {
                        log::error!("Remote command failed with stderr: {}", stderr);
                        // Create a JSON error
                        let error_json = format!(
                            "{{\"type\":\"error\",\"message\":\"{}\",\"code\":{}}}",
                            stderr.replace('\"', "\\\"").replace('\n', "\\n"),
                            status.code().unwrap_or(1)
                        );

                        // Output error JSON and return it
                        println!("{}", error_json);
                        Ok(error_json)
                    } else {
                        log::error!("Remote command failed with output: {}", output);
                        // Create a JSON error from the output
                        let error_json = format!(
                            "{{\"type\":\"error\",\"message\":\"{}\",\"code\":{}}}",
                            output.replace('\"', "\\\"").replace('\n', "\\n"),
                            status.code().unwrap_or(1)
                        );

                        // Output error JSON and return it
                        println!("{}", error_json);
                        Ok(error_json)
                    }
                }
            } else {
                // Check if output is valid JSON
                if Self::is_valid_json(&output) {
                    log::debug!("Received valid JSON from remote dedups");
                    Ok(output)
                } else if let Some(json_only) = Self::extract_json_lines(&output) {
                    log::debug!(
                        "Extracted {} JSON lines from mixed output",
                        json_only.lines().count()
                    );
                    Ok(json_only)
                } else {
                    log::warn!("Remote output did not contain JSON, creating error response");
                    let error_json = "{\"type\":\"error\",\"message\":\"Remote output not valid JSON\",\"code\":1}".to_string();
                    println!("{}", error_json);
                    Ok(error_json)
                }
            }
        } else {
            // Standard non-JSON execution
            let output = std::process::Command::new(&ssh_cmd[0])
                .args(&ssh_cmd[1..])
                .output()
                .with_context(|| {
                    format!("Failed to execute command on host '{}'", self.remote.host)
                })?;

            if !output.status.success() {
                let stderr = String::from_utf8_lossy(&output.stderr);
                let stdout = String::from_utf8_lossy(&output.stdout);

                if !stderr.is_empty() {
                    log::error!("Remote command failed with stderr: {}", stderr);
                    Err(anyhow::anyhow!("Remote dedups command failed: {}", stderr))
                } else {
                    log::error!("Remote command failed with output: {}", stdout);
                    Err(anyhow::anyhow!("Remote dedups command failed: {}", stdout))
                }
            } else {
                let stdout = String::from_utf8_lossy(&output.stdout);
                log::debug!("Command output: {}", stdout);

                Ok(stdout.into_owned())
            }
        }
    }

    /// Check if a string contains valid JSON
    fn is_valid_json(text: &str) -> bool {
        // Check for some JSON structures
        text.trim().starts_with('{') && text.trim().ends_with('}') && text.contains("\"type\":")
    }

    /// Close the SSH connection
    pub fn close(&mut self) {
        if self.session.is_some() {
            // Clean up any SSH tunnels we created
            let cleanup_cmd = format!("pkill -f 'ssh.*-L.*{}.*-N'", self.remote.port.unwrap_or(22));
            if let Err(e) = std::process::Command::new("sh")
                .arg("-c")
                .arg(&cleanup_cmd)
                .output()
            {
                log::warn!("Failed to clean up SSH tunnel: {}", e);
            }
        }
        self.session = None;
    }

    /// Get SSH options from CLI, remote location, and default settings
    pub fn get_ssh_options(&self, cli: &crate::Cli) -> Vec<String> {
        let mut options = Vec::new();

        // Add options from the CLI
        #[cfg(feature = "ssh")]
        {
            options.extend(cli.ssh_options.clone());
        }

        // Add SSH config file option if found in regular locations
        if Path::new("/Users/anant/.ssh/config").exists() {
            options.push("-F".to_string());
            options.push("/Users/anant/.ssh/config".to_string());
        } else if Path::new("/etc/ssh/ssh_config").exists() {
            options.push("-F".to_string());
            options.push("/etc/ssh/ssh_config".to_string());
        }

        // Add default options for better SSH experience
        options.push("-o".to_string());
        options.push("BatchMode=yes".to_string());

        // Add SSH config file from environment variable if set
        if let Ok(config_file) = std::env::var("SSH_CONFIG_FILE") {
            if !config_file.is_empty() {
                log::debug!("Using SSH config file from environment: {}", config_file);
                options.push("-F".to_string());
                options.push(config_file);
            }
        }

        options
    }
}

/// Dummy implementation for non-SSH builds
#[cfg(not(feature = "ssh"))]
#[derive(Debug, Clone)]
pub struct RemoteLocation;

#[cfg(not(feature = "ssh"))]
impl RemoteLocation {
    pub fn parse(_location_str: &str) -> Result<Self, anyhow::Error> {
        Err(anyhow::anyhow!("SSH support is not enabled in this build"))
    }

    pub fn is_ssh_path(_path: &str) -> bool {
        false
    }
}
