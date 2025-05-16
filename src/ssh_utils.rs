#[cfg(feature = "ssh")]
use anyhow::{Context, Result};
#[cfg(feature = "ssh")]
use ssh2::Session;
#[cfg(feature = "ssh")]
use std::path::{Path, PathBuf};
#[cfg(feature = "ssh")]
use std::process::Command;

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
    pub local_tunnel_port: Option<u16>,
    pub remote_tunnel_port: Option<u16>,
}

#[cfg(feature = "ssh")]
impl RemoteLocation {
    /// Parses SSH format strings like:
    /// - `ssh:host:/path`
    /// - `ssh:user@host:/path`
    /// - `ssh:user@host:port:/path`
    /// - `ssh:host:/path:ssh_opt1,ssh_opt2:rsync_opt1,rsync_opt2`
    /// - `ssh:host:/path:ssh_opt1,ssh_opt2:rsync_opt1,rsync_opt2:local_port:remote_port`
    pub fn parse(location_str: &str) -> Result<Self> {
        // Check if it starts with ssh:
        if !location_str.starts_with("ssh:") {
            return Err(anyhow::anyhow!("Not an SSH location: {}", location_str));
        }

        // Remove the ssh: prefix
        let without_prefix = &location_str[4..];

        // Split the path by colon to handle the various parts
        let parts: Vec<&str> = without_prefix.splitn(7, ':').collect();

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
        let mut local_tunnel_port = None;
        let mut remote_tunnel_port = None;

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

        // Parse optional tunnel ports
        if parts.len() > path_idx + 3 && !parts[path_idx + 3].is_empty() {
            local_tunnel_port = Some(parts[path_idx + 3].parse::<u16>()?);
        }

        if parts.len() > path_idx + 4 && !parts[path_idx + 4].is_empty() {
            remote_tunnel_port = Some(parts[path_idx + 4].parse::<u16>()?);
        }

        Ok(RemoteLocation {
            user,
            host,
            port,
            path,
            ssh_options,
            rsync_options,
            local_tunnel_port,
            remote_tunnel_port,
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
    pub async fn check_dedups_installed(&self) -> Result<Option<String>> {
        log::info!("Checking if dedups is installed on remote host '{}'...", self.host);
        
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
                let output = output.trim();
                if output != "not found" {
                    log::info!("Found dedups on remote host '{}' at: {}", self.host, output);
                    Ok(Some(output.to_string()))
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
        log::info!("Attempting to install dedups on remote host '{}'...", self.host);
        
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
            format!("sudo mkdir -p {} && sudo chown $USER {}", install_dir, install_dir)
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
                            log::info!("Successfully installed dedups on remote host '{}' at: {}", self.host, path);
                            Ok(path.to_string())
                        } else {
                            log::error!("Installation appeared to succeed but dedups not found in PATH");
                            Err(anyhow::anyhow!("Installation verification failed: dedups not found in PATH"))
                        }
                    }
                    Err(e) => {
                        log::error!("Failed to verify installation: {}", e);
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
    local_port: Option<u16>,
    command_channel: Option<ssh2::Channel>,
}

#[cfg(feature = "ssh")]
impl std::fmt::Debug for SshProtocol {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SshProtocol")
            .field("remote", &self.remote)
            .field("session", &(self.session.is_some()))
            .field("local_port", &self.local_port)
            .field("command_channel", &self.command_channel.is_some())
            .finish()
    }
}

#[cfg(feature = "ssh")]
impl Clone for SshProtocol {
    fn clone(&self) -> Self {
        // We can't clone the session or channel, so create a new one without them
        SshProtocol {
            session: None,
            remote: self.remote.clone(),
            local_port: None,
            command_channel: None,
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
            local_port: None,
            command_channel: None,
        }
    }
    
    /// Connect to the remote host
    pub fn connect(&mut self) -> Result<()> {
        log::debug!("Connecting to remote host '{}'...", self.remote.host);
        
        // Build SSH command
        let mut ssh_cmd = vec!["ssh".to_string()];
        
        // Add port if specified
        if let Some(port) = self.remote.port {
            ssh_cmd.extend(vec!["-p".to_string(), port.to_string()]);
        }
        
        // Add custom SSH options
        ssh_cmd.extend(self.remote.ssh_options.clone());
        
        // Add host with optional user
        let host = if let Some(user) = &self.remote.user {
            format!("{}@{}", user, self.remote.host)
        } else {
            self.remote.host.clone()
        };
        ssh_cmd.push(host);
        
        // Test SSH connection first
        let test_cmd = format!("{} echo 'SSH connection test'", ssh_cmd.join(" "));
        let output = Command::new("sh")
            .arg("-c")
            .arg(&test_cmd)
            .output()
            .with_context(|| format!("Failed to test SSH connection to host '{}'", self.remote.host))?;
            
        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(anyhow::anyhow!(
                "SSH connection test failed: {}",
                stderr
            ));
        }
        
        // Create new SSH session
        let mut sess = Session::new()?;
        
        // Connect using standard TCP
        let tcp = std::net::TcpStream::connect(format!("{}:22", self.remote.host))
            .with_context(|| format!("Failed to connect to host '{}'", self.remote.host))?;
        sess.set_tcp_stream(tcp);
        
        // Perform SSH handshake
        sess.handshake()
            .with_context(|| format!("SSH handshake failed with host '{}'", self.remote.host))?;
        
        // Try to authenticate with SSH agent first
        match sess.userauth_agent(self.remote.user.as_deref().unwrap_or("")) {
            Ok(_) => {
                log::debug!("Successfully authenticated with SSH agent");
                self.session = Some(sess);
                Ok(())
            },
            Err(_) => {
                // Fallback to pubkey authentication
                let mut auth_success = false;
                
                // Try with default key locations
                for key_file in get_default_key_files() {
                    if key_file.exists() {
                        match sess.userauth_pubkey_file(
                            self.remote.user.as_deref().unwrap_or(""),
                            None,
                            &key_file,
                            None,
                        ) {
                            Ok(_) => {
                                log::debug!("Successfully authenticated with key file: {:?}", key_file);
                                auth_success = true;
                                self.session = Some(sess);
                                break;
                            }
                            Err(e) => {
                                log::debug!("Failed to authenticate with key file {:?}: {}", key_file, e);
                            }
                        }
                    }
                }
                
                if !auth_success {
                    Err(anyhow::anyhow!(
                        "Authentication failed for host '{}'. Please verify:\n\
                        1. SSH key is properly set up\n\
                        2. The key is added to ssh-agent or specified in config\n\
                        3. The remote user has the correct permissions",
                        self.remote.host
                    ))
                } else {
                    Ok(())
                }
            }
        }
    }
    
    /// Execute dedups command on remote system using the SSH command line
    pub fn execute_dedups(&self, args: &[&str]) -> Result<String> {
        // Build SSH command
        let mut ssh_cmd = vec!["ssh".to_string()];
        
        // Add port if specified
        if let Some(port) = self.remote.port {
            ssh_cmd.extend(vec!["-p".to_string(), port.to_string()]);
        }
        
        // Add custom SSH options
        ssh_cmd.extend(self.remote.ssh_options.clone());
        
        // Add host with optional user
        let host = if let Some(user) = &self.remote.user {
            format!("{}@{}", user, self.remote.host)
        } else {
            self.remote.host.clone()
        };
        ssh_cmd.push(host);
        
        // Set up environment and command
        let setup_env = r#"
            export PATH="$HOME/.local/bin:$PATH"
            if [ -f "$HOME/.bashrc" ]; then
                source "$HOME/.bashrc"
            fi
            if [ -f "$HOME/.profile" ]; then
                source "$HOME/.profile"
            fi
        "#;
        
        // Combine setup and command
        let command = format!(
            "{}\ndedups {}",
            setup_env,
            args.join(" ")
        );
        
        ssh_cmd.push(command);
        
        log::debug!("Executing remote command: {}", ssh_cmd.join(" "));
        
        let output = Command::new(&ssh_cmd[0])
            .args(&ssh_cmd[1..])
            .output()
            .with_context(|| format!("Failed to execute command on host '{}'", self.remote.host))?;
            
        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            if !stderr.is_empty() {
                return Err(anyhow::anyhow!(
                    "Remote dedups command failed: {}",
                    stderr
                ));
            } else {
                let stdout = String::from_utf8_lossy(&output.stdout);
                return Err(anyhow::anyhow!(
                    "Remote dedups command failed: {}",
                    stdout
                ));
            }
        }
        
        Ok(String::from_utf8_lossy(&output.stdout).into_owned())
    }
    
    /// Close the SSH connection
    pub fn close(&mut self) {
        self.command_channel = None;
        self.session = None;
        self.local_port = None;
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

#[cfg(feature = "ssh")]
fn find_unused_port() -> Result<u16> {
    // Try to bind to port 0 which lets the OS assign an available port
    let listener = std::net::TcpListener::bind("127.0.0.1:0")?;
    let port = listener.local_addr()?.port();
    drop(listener); // Immediately close the listener
    Ok(port)
}

#[cfg(all(test, feature = "ssh"))]
mod tests {
    use super::*;

    #[test]
    fn test_remote_location_parse() {
        // Test basic format
        let loc = RemoteLocation::parse("ssh:host:/path").unwrap();
        assert_eq!(loc.host, "host");
        assert_eq!(loc.path.to_str().unwrap(), "/path");
        assert_eq!(loc.user, None);
        assert_eq!(loc.port, None);
        
        // Test with username
        let loc = RemoteLocation::parse("ssh:user@host:/path").unwrap();
        assert_eq!(loc.host, "host");
        assert_eq!(loc.user.unwrap(), "user");
        assert_eq!(loc.path.to_str().unwrap(), "/path");
        
        // Test with port
        let loc = RemoteLocation::parse("ssh:host:2222:/path").unwrap();
        assert_eq!(loc.host, "host");
        assert_eq!(loc.port.unwrap(), 2222);
        
        // Test with options
        let loc = RemoteLocation::parse("ssh:host:/path:opt1,opt2:rsync1,rsync2").unwrap();
        assert_eq!(loc.ssh_options, vec!["opt1", "opt2"]);
        assert_eq!(loc.rsync_options, vec!["rsync1", "rsync2"]);
    }

    #[test]
    fn test_ssh_protocol_lifecycle() {
        // Skip test if SSH to localhost is not available
        if std::process::Command::new("ssh")
            .args(["-q", "localhost", "echo", "test"])
            .output()
            .is_ok()
        {
            let mut protocol = SshProtocol::new(
                RemoteLocation::parse("ssh:localhost:/tmp").unwrap()
            );
            
            assert!(protocol.session.is_none());
            assert!(protocol.command_channel.is_none());
            
            // Test connection
            if let Err(e) = protocol.connect() {
                println!("SSH connection failed: {}", e);
            } else {
                assert!(protocol.session.is_some());
                
                // Test command execution
                let result = protocol.execute_dedups(&["--version"]);
                assert!(result.is_err()); // Should fail as dedups might not be installed
                
                // Test cleanup
                protocol.close();
                assert!(protocol.session.is_none());
                assert!(protocol.command_channel.is_none());
            }
        }
    }

    #[test]
    fn test_ssh_command_generation() {
        let loc = RemoteLocation::parse("ssh:user@host:2222:/path:opt1,opt2").unwrap();
        let mut protocol = SshProtocol::new(loc);
        
        // Test that we can build a command (but don't execute it)
        let result = protocol.connect();
        assert!(result.is_err()); // Should fail as host doesn't exist
    }
} 