# WireGuard Failover Installer - Usage Examples

## Overview

The refactored `install.py` script now supports remote installation without copying the install script itself to the remote server. Instead, it:
1. Copies only the necessary assets (binary, config, service file) to the remote server
2. Generates and executes a remote installation script via SSH
3. Cleans up temporary files after installation

## Prerequisites

### For Local Installation
- Root privileges (sudo)
- Systemd-based Linux distribution
- NetworkManager or systemd-networkd

### For Remote Installation
- SSH access to the remote server
- Private key authentication
- Sudo privileges on the remote server
- Python 3 on the local machine (for running the installer)

## Basic Usage

### Local Installation
```bash
# Install locally (requires root)
sudo python3 install.py --local

# Update existing installation
sudo python3 install.py --update
```

### Remote Installation
```bash
# Basic remote installation
python3 install.py --remote --target-ip 192.168.1.100 --private-key ~/.ssh/id_rsa

# Remote installation with custom username
python3 install.py --remote --target-ip 192.168.1.100 --private-key ~/.ssh/id_rsa --username admin

# Remote update
python3 install.py --remote --target-ip 192.168.1.100 --private-key ~/.ssh/id_rsa --update
```

## Complete Examples

### Example 1: Full Remote Installation with All Options
```bash
python3 install.py \
  --remote \
  --target-ip 203.0.113.45 \
  --private-key /path/to/private_key.pem \
  --username ubuntu \
  --sudo-password "your_sudo_password"
```

### Example 2: Interactive Remote Installation (password prompted)
```bash
python3 install.py \
  --remote \
  --target-ip 192.168.1.100 \
  --private-key ~/.ssh/id_ed25519 \
  --username root
# You will be prompted for the sudo password
```

### Example 3: Update Both Local and Remote Systems
```bash
# First update locally
sudo python3 install.py --update

# Then update remote server
python3 install.py --remote --target-ip 192.168.1.100 --private-key ~/.ssh/id_rsa --update
```

## What Gets Copied to Remote Server

The installer copies ONLY these files to the remote server:
- `wg-failover` binary (from `target/release/` or current directory)
- `wg-failover.service` systemd service file
- `config.toml` configuration file

The `install.py` script itself is NOT copied to the remote server.

## Installation Process Details

### Remote Installation Steps
1. **Connection Test**: Verifies SSH connectivity to the remote server
2. **File Transfer**: Copies only the necessary assets to a temporary directory on the remote server
3. **Script Generation**: Creates a remote installation script with the appropriate logic
4. **Execution**: Runs the installation script on the remote server with sudo privileges
5. **Cleanup**: Removes all temporary files from both local and remote systems
6. **Verification**: Checks service status on the remote server

### What the Remote Script Does
1. Stops the existing service (if running)
2. Backs up existing configuration (for updates)
3. Installs the binary to `/usr/local/bin/wg-failover`
4. Installs the systemd service to `/etc/systemd/system/wg-failover.service`
5. Installs configuration to `/etc/wg-failover/config.toml`
6. Starts and enables the service
7. Verifies the installation

## Troubleshooting

### Common Issues

1. **SSH Connection Failed**
   ```
   Error: Could not connect to 192.168.1.100 via SSH
   ```
   - Verify the target IP is correct and reachable
   - Check that the private key file exists and has correct permissions (600)
   - Ensure SSH service is running on the remote server

2. **Permission Denied on Remote Server**
   ```
   Error: This script must be run as root
   ```
   - The remote user needs sudo privileges
   - Provide the correct sudo password when prompted

3. **Missing Files**
   ```
   Error: Required file 'wg-failover.service' not found
   ```
   - Ensure you're running the script from the project directory
   - Build the binary first: `cargo build --release`

4. **Service Failed to Start**
   - Check logs on remote server: `journalctl -u wg-failover.service`
   - Verify WireGuard is installed and configured on the remote server
   - Check that the network interfaces specified in config.toml exist

### Debug Mode

For troubleshooting, you can add debug output by modifying the remote installation script. The script is generated in the temporary directory on the remote server at `/tmp/wg-failover-install/remote_install.sh`.

## Security Considerations

1. **Private Keys**: Keep your SSH private keys secure with 600 permissions
2. **Sudo Passwords**: Avoid passing passwords on the command line when possible (use interactive prompt)
3. **Temporary Files**: All temporary files are automatically cleaned up after installation
4. **Service Account**: The service runs as root (required for network operations)

## Building the Binary

Before installation, ensure the binary is built:
```bash
# Build release binary
cargo build --release

# The binary will be at: target/release/wg-failover
```

## Configuration

After installation, edit the configuration file:
- Local: `/etc/wg-failover/config.toml`
- Remote: Same path on the remote server

Key configuration sections:
- `[peer]`: WireGuard peer IP to monitor
- `[wireguard]`: WireGuard interface name
- `[interfaces]`: Primary and secondary network interfaces
- `[monitoring]`: Timing and behavior settings

## Service Management

### On Local System
```bash
# Start service
sudo systemctl start wg-failover.service

# Stop service
sudo systemctl stop wg-failover.service

# Check status
sudo systemctl status wg-failover.service

# View logs
sudo journalctl -u wg-failover.service -f
```

### On Remote System (via SSH)
```bash
# Check remote service status
ssh -i ~/.ssh/id_rsa user@remote-server "sudo systemctl status wg-failover.service"

# View remote logs
ssh -i ~/.ssh/id_rsa user@remote-server "sudo journalctl -u wg-failover.service -n 50"
```

## Uninstallation

### Local Uninstallation
```bash
sudo systemctl stop wg-failover.service
sudo systemctl disable wg-failover.service
sudo rm -f /usr/local/bin/wg-failover
sudo rm -f /etc/systemd/system/wg-failover.service
sudo rm -rf /etc/wg-failover
sudo rm -f /var/log/wg-failover.log
```

### Remote Uninstallation
Create an uninstall script and run it via SSH, or manually execute the commands above on the remote server.

## Notes

- The installer preserves existing configuration during updates
- Network interface names should match your system's actual interfaces
- The service requires CAP_NET_ADMIN and CAP_NET_RAW capabilities
- Logs are written to both journalctl and `/var/log/wg-failover.log`
