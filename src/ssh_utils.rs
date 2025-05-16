#[cfg(feature = "ssh")]
use anyhow::{Context, Result};
#[cfg(feature = "ssh")]
use ssh2::Session;
#[cfg(feature = "ssh")]
use std::io::Read;
#[cfg(feature = "ssh")]
use std::net::TcpStream;
#[cfg(feature = "ssh")]
use std::path::{Path, PathBuf};

/// Represents a remote location parsed from an SSH URI
#[cfg(feature = "ssh")]
#[derive(Debug, Clone)]
pub struct RemoteLocation {
    pub user: Option<String>,
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
        let (user, host) = if host_part.contains('@') {
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
            user,
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
        let host = if let Some(user) = &self.user {
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
        let host_prefix = if let Some(user) = &self.user {
            format!("{}@{}", user, self.host)
        } else {
            self.host.clone()
        };
        
        let source_str = if is_remote_source {
            format!("{}:{}", host_prefix, source.display())
        } else {
            source.display().to_string()
        };
        
        let dest_str = if !is_remote_source {
            format!("{}:{}", host_prefix, dest.display())
        } else {
            dest.display().to_string()
        };
        
        cmd.push(source_str);
        cmd.push(dest_str);
        
        cmd
    }

    /// Run a command on the remote system
    pub fn run_command(&self, command: &str) -> Result<String> {
        // Build SSH command with proper options
        let mut ssh_cmd = vec!["ssh".to_string()];
        
        // Add port if specified
        if let Some(port) = self.port {
            ssh_cmd.extend(vec!["-p".to_string(), port.to_string()]);
        }
        
        // Always use system SSH config by default
        ssh_cmd.extend(vec!["-F".to_string(), shellexpand::tilde("~/.ssh/config").into_owned()]);
        
        // Add any custom SSH options
        ssh_cmd.extend(self.ssh_options.clone());
        
        // Add host with optional user
        let host = if let Some(user) = &self.user {
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
        
        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            let stdout = String::from_utf8_lossy(&output.stdout);
            log::error!("SSH command failed. Stderr: {}", stderr);
            log::error!("SSH command stdout: {}", stdout);
            return Err(anyhow::anyhow!(
                "SSH command failed on host '{}': {}",
                self.host,
                stderr
            ));
        }
        
        Ok(String::from_utf8_lossy(&output.stdout).into_owned())
    }
    
    /// Check if dedups is installed on the remote system
    pub async fn check_dedups_installed(&self) -> Result<bool> {
        log::info!("Checking if dedups is installed on remote host '{}'...", self.host);
        let output = self.run_command("which dedups 2>/dev/null || echo 'not found'")?;
        let installed = !output.contains("not found");
        if installed {
            let path = output.trim();
            log::info!("Found dedups on remote host '{}' at: {}", self.host, path);
        } else {
            log::info!("dedups not found on remote host '{}'", self.host);
        }
        Ok(installed)
    }
    
    /// Install dedups on the remote system
    pub fn install_dedups(&self) -> Result<()> {
        log::info!("Attempting to install dedups on remote host '{}'...", self.host);
        
        // First check if we have sudo access
        let has_sudo = self.run_command("sudo -n true 2>/dev/null && echo 'yes' || echo 'no'")?;
        let install_dir = if has_sudo.trim() == "yes" {
            log::info!("Sudo access available on remote host, will install to /usr/local/bin");
            "/usr/local/bin"
        } else {
            log::info!("No sudo access on remote host, will install to user's ~/.local/bin");
            "~/.local/bin"
        };
        
        // Create install directory if it doesn't exist
        let mkdir_cmd = format!("mkdir -p {}", install_dir);
        log::debug!("Creating installation directory: {}", install_dir);
        self.run_command(&mkdir_cmd)?;
        
        // Download and install dedups
        let install_cmd = format!(
            "curl -sSL https://raw.githubusercontent.com/AtlasPilotPuppy/dedup/main/install.sh | {} bash -s -- --ssh",
            if has_sudo.trim() == "yes" { "sudo" } else { "" }
        );
        
        log::info!("Downloading and installing dedups on remote host...");
        match self.run_command(&install_cmd) {
            Ok(_) => {
                // Verify installation
                let verify_cmd = "which dedups";
                match self.run_command(verify_cmd) {
                    Ok(path) => {
                        log::info!("Successfully installed dedups on remote host '{}' at: {}", self.host, path.trim());
                        Ok(())
                    }
                    Err(e) => {
                        log::error!("Installation appeared to succeed but dedups not found in PATH. Error: {}", e);
                        Err(anyhow::anyhow!("Installation verification failed: {}", e))
                    }
                }
            }
            Err(e) => {
                log::error!("Failed to install dedups on remote host '{}': {}", self.host, e);
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
        log::info!("Checking if remote directory exists: {}", self.path.display());
        
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
                    self.host, self.host, e
                ));
            }
        }

        // Then check if the directory exists
        let check_cmd = format!("test -d '{}' && echo 'EXISTS' || echo 'NOTFOUND'", self.path.display());
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

/// Get default SSH key file locations
#[cfg(feature = "ssh")]
fn get_default_key_files() -> Vec<PathBuf> {
    let home = match dirs::home_dir() {
        Some(home) => home,
        None => return Vec::new(),
    };
    
    let ssh_dir = home.join(".ssh");
    
    vec![
        ssh_dir.join("id_rsa"),
        ssh_dir.join("id_dsa"),
        ssh_dir.join("id_ecdsa"),
        ssh_dir.join("id_ed25519"),
    ]
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
        let tcp = TcpStream::connect(format!(
            "{}:{}",
            self.remote.host,
            self.remote.port.unwrap_or(22)
        ))
        .context("Failed to connect to remote host")?;
        
        let mut sess = Session::new()?;
        sess.set_tcp_stream(tcp);
        sess.handshake()?;
        
        // Try to authenticate with SSH agent first
        match sess.userauth_agent(self.remote.user.as_deref().unwrap_or("")) {
            Ok(_) => (),
            Err(_) => {
                // Fallback to pubkey authentication
                let mut tried_keys = false;
                
                // Try with default key locations
                for key_file in get_default_key_files() {
                    if key_file.exists() {
                        if let Ok(_) = sess.userauth_pubkey_file(
                            self.remote.user.as_deref().unwrap_or(""),
                            None,
                            &key_file,
                            None,
                        ) {
                            tried_keys = true;
                            break;
                        }
                    }
                }
                
                // If no key authentication worked, return error
                if !tried_keys {
                    return Err(anyhow::anyhow!("Authentication failed"));
                }
            }
        }
        
        self.session = Some(sess);
        Ok(())
    }
    
    /// Execute dedups command on remote system
    pub fn execute_dedups(&self, args: &[&str]) -> Result<String> {
        let sess = self.session.as_ref().ok_or_else(|| anyhow::anyhow!("Not connected"))?;
        
        let command = format!("dedups {}", args.join(" "));
        
        let mut channel = sess.channel_session()?;
        channel.exec(&command)?;
        
        let mut output = String::new();
        channel.read_to_string(&mut output)?;
        
        channel.wait_close()?;
        let exit_status = channel.exit_status()?;
        
        if exit_status != 0 {
            let mut stderr = String::new();
            channel.stderr().read_to_string(&mut stderr)?;
            
            if !stderr.is_empty() {
                return Err(anyhow::anyhow!(
                    "Dedups command failed with status {}: {}",
                    exit_status,
                    stderr
                ));
            }
        }
        
        Ok(output)
    }
    
    /// Close the SSH connection
    pub fn close(&mut self) {
        self.session = None;
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