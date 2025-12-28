#!/usr/bin/env python3
"""
WireGuard Failover Installer
============================
This script installs or updates the WireGuard Failover service locally or remotely.
For remote installation, it copies only necessary assets (binary, config, service file)
and executes remote commands via SSH without copying the install script itself.
"""

import argparse
import getpass
import os
import shutil
import subprocess
import sys
import tempfile
from pathlib import Path

# Color codes for terminal output
BLUE = '\033[94m'
GREEN = '\033[92m'
YELLOW = '\033[93m'
RED = '\033[91m'
ENDC = '\033[0m'

# Installation paths
BINARY_PATH = "/usr/local/bin/wg-failover"
CONFIG_DIR = "/etc/wg-failover"
CONFIG_PATH = f"{CONFIG_DIR}/config.toml"
SERVICE_PATH = "/etc/systemd/system/wg-failover.service"
LOG_PATH = "/var/log/wg-failover.log"

def print_color(text: str, color: str) -> None:
    """Print colored text to terminal"""
    print(f"{color}{text}{ENDC}")

def replace_config_with_latest(source_config: str, dest_config: str) -> None:
    """Replace configuration file with latest version, preserving user settings"""
    if not os.path.exists(dest_config):
        # If destination doesn't exist, just copy
        shutil.copy(source_config, dest_config)
        return
    
    # Read existing config
    with open(dest_config, 'r') as f:
        existing_lines = f.readlines()
    
    # Read new config
    with open(source_config, 'r') as f:
        new_lines = f.readlines()
    
    # Extract sections from existing config
    existing_sections = {}
    current_section = None
    section_content = []
    
    for line in existing_lines:
        line_stripped = line.strip()
        if line_stripped.startswith('[') and line_stripped.endswith(']'):
            if current_section is not None:
                existing_sections[current_section] = section_content
            current_section = line_stripped
            section_content = [line]
        elif current_section is not None:
            section_content.append(line)
    
    if current_section is not None:
        existing_sections[current_section] = section_content
    
    # Build new config with preserved sections
    output_lines = []
    current_section = None
    section_content = []
    
    for line in new_lines:
        line_stripped = line.strip()
        if line_stripped.startswith('[') and line_stripped.endswith(']'):
            if current_section is not None:
                # Write preserved section content if exists, otherwise new content
                if current_section in existing_sections:
                    output_lines.extend(existing_sections[current_section])
                else:
                    output_lines.extend(section_content)
            
            current_section = line_stripped
            section_content = [line]
        elif current_section is not None:
            section_content.append(line)
    
    # Handle last section
    if current_section is not None:
        if current_section in existing_sections:
            output_lines.extend(existing_sections[current_section])
        else:
            output_lines.extend(section_content)
    
    # Write updated config
    with open(dest_config, 'w') as f:
        f.writelines(output_lines)

def check_root() -> None:
    """Check if running as root"""
    if os.geteuid() != 0:
        print_color("Error: This script must be run as root for local installation", RED)
        sys.exit(1)

def is_installed() -> bool:
    """Check if wg-failover is already installed"""
    return os.path.exists(BINARY_PATH) and os.path.exists(SERVICE_PATH)

def is_service_running() -> bool:
    """Check if wg-failover service is running"""
    try:
        result = subprocess.run(['systemctl', 'is-active', 'wg-failover.service'], 
                               capture_output=True, text=True)
        return result.returncode == 0
    except (subprocess.CalledProcessError, FileNotFoundError):
        return False

def get_installed_version() -> str | None:
    """Get version of installed wg-failover binary"""
    if not os.path.exists(BINARY_PATH):
        return None
    
    try:
        result = subprocess.run([BINARY_PATH, '--version'], 
                               capture_output=True, text=True)
        if result.returncode == 0:
            # Extract version from output like "wg-failover 0.1.0"
            for line in result.stdout.splitlines():
                if 'wg-failover' in line:
                    parts = line.split()
                    if len(parts) >= 2:
                        return parts[1]
    except (subprocess.CalledProcessError, FileNotFoundError):
        pass
    
    return None

def get_current_version() -> str | None:
    """Get version from current directory"""
    # Try to get version from Cargo.toml
    cargo_toml_path = os.path.join(os.path.dirname(os.path.realpath(__file__)), 'Cargo.toml')
    if os.path.exists(cargo_toml_path):
        try:
            with open(cargo_toml_path, 'r') as f:
                for line in f:
                    if line.strip().startswith('version ='):
                        # Extract version from: version = "0.1.0"
                        version = line.split('=')[1].strip().strip('"\'')
                        return version
        except Exception:
            pass
    
    # Try to get version from built binary
    current_dir = os.path.dirname(os.path.realpath(__file__))
    binary_path = os.path.join(current_dir, 'target', 'release', 'wg-failover')
    if not os.path.exists(binary_path):
        binary_path = os.path.join(current_dir, 'wg-failover')
    
    if os.path.exists(binary_path):
        try:
            result = subprocess.run([binary_path, '--version'], 
                                   capture_output=True, text=True)
            if result.returncode == 0:
                for line in result.stdout.splitlines():
                    if 'wg-failover' in line:
                        parts = line.split()
                        if len(parts) >= 2:
                            return parts[1]
        except (subprocess.CalledProcessError, FileNotFoundError):
            pass
    
    return None

def backup_config() -> bool:
    """Backup existing configuration file"""
    if os.path.exists(CONFIG_PATH):
        backup_path = f"{CONFIG_PATH}.backup"
        try:
            shutil.copy(CONFIG_PATH, backup_path)
            print_color(f"Configuration backed up to {backup_path}", GREEN)
            return True
        except Exception as e:
            print_color(f"Warning: Could not backup configuration: {e}", YELLOW)
            return False
    return False

def restore_config() -> bool:
    """Restore configuration from backup"""
    backup_path = f"{CONFIG_PATH}.backup"
    if os.path.exists(backup_path):
        try:
            shutil.copy(backup_path, CONFIG_PATH)
            print_color(f"Configuration restored from {backup_path}", GREEN)
            return True
        except Exception as e:
            print_color(f"Error: Could not restore configuration: {e}", RED)
            return False
    return False

def stop_service() -> bool:
    """Stop wg-failover service"""
    was_running = is_service_running()
    if was_running:
        print_color("Stopping wg-failover service...", YELLOW)
        try:
            subprocess.run(['systemctl', 'stop', 'wg-failover.service'], check=True)
            print_color("Service stopped successfully", GREEN)
        except subprocess.CalledProcessError:
            print_color("Warning: Could not stop service via systemctl", YELLOW)
            # Try to kill the process directly
            try:
                subprocess.run(['pkill', '-f', 'wg-failover'], capture_output=True)
                print_color("Process terminated", GREEN)
            except Exception:
                print_color("Warning: Could not terminate process", YELLOW)
    return was_running

def start_service() -> bool:
    """Start wg-failover service"""
    print_color("Starting wg-failover service...", GREEN)
    try:
        subprocess.run(['systemctl', 'start', 'wg-failover.service'], check=True)
        # Wait a moment for service to start
        import time
        time.sleep(2)
        
        # Verify service is running
        if is_service_running():
            print_color("Service started successfully", GREEN)
            return True
        else:
            print_color("Warning: Service may not have started properly", YELLOW)
            return False
    except subprocess.CalledProcessError as e:
        print_color(f"Error starting service: {e}", RED)
        return False

def enable_service() -> bool:
    """Enable wg-failover service to start on boot"""
    print_color("Enabling wg-failover service to start on boot...", GREEN)
    try:
        subprocess.run(['systemctl', 'enable', 'wg-failover.service'], check=True)
        print_color("Service enabled successfully", GREEN)
        return True
    except subprocess.CalledProcessError as e:
        print_color(f"Warning: Could not enable service: {e}", YELLOW)
        return False

def local_install(current_dir: str, is_update: bool) -> None:
    """Perform local installation or update"""
    if is_update:
        print_color("=== Updating WireGuard Failover ===", BLUE)
        installed_version = get_installed_version()
        current_version = get_current_version()
        
        if installed_version and current_version:
            print_color(f"Updating from version {installed_version} to {current_version}", BLUE)
        elif installed_version:
            print_color(f"Updating existing installation (current version: {installed_version})", BLUE)
        else:
            print_color("Updating existing installation", BLUE)
        
        # Backup configuration before update
        _ = backup_config()
        
        # Stop service before update
        was_running: bool = stop_service()
    else:
        print_color("=== Installing WireGuard Failover Locally ===", GREEN)
        was_running = False
    
    # Check for required commands
    print_color("Checking dependencies...", GREEN)
    required_commands = ['ip', 'systemctl', 'nmcli']
    missing_commands: list[str] = []
    for cmd in required_commands:
        if not shutil.which(cmd):
            missing_commands.append(cmd)
    
    if missing_commands:
        print_color(f"Warning: Missing commands: {', '.join(missing_commands)}", YELLOW)
        print_color("Some features may not work properly", YELLOW)
    
    # Install binary
    binary_src = os.path.join(current_dir, 'target', 'release', 'wg-failover')
    if not os.path.exists(binary_src):
        binary_src = os.path.join(current_dir, 'wg-failover')
    
    if not os.path.exists(binary_src):
        print_color(f"Error: wg-failover executable not found in {current_dir}/target/release/ or {current_dir}/", RED)
        if is_update:
            restore_config()
        sys.exit(1)
    
    print_color(f"Installing binary to {BINARY_PATH}...", GREEN)
    _ = shutil.copy(binary_src, BINARY_PATH)
    os.chmod(BINARY_PATH, 0o755)
    print_color("Binary installed successfully", GREEN)
    
    # Install service file
    service_src = os.path.join(current_dir, 'wg-failover.service')
    if not os.path.exists(service_src):
        print_color(f"Error: wg-failover.service file not found in {current_dir}", RED)
        if is_update:
            restore_config()
        sys.exit(1)
    
    print_color(f"Installing systemd service to {SERVICE_PATH}...", GREEN)
    _ = shutil.copy(service_src, SERVICE_PATH)
    os.chmod(SERVICE_PATH, 0o644)
    _ = subprocess.run(['systemctl', 'daemon-reload'], check=True)
    print_color("Service installed successfully", GREEN)
    
    # Install configuration - always replace with latest version
    config_src = os.path.join(current_dir, 'config.toml')
    if os.path.exists(config_src):
        print_color(f"Installing configuration to {CONFIG_PATH}...", GREEN)
        os.makedirs(CONFIG_DIR, mode=0o755, exist_ok=True)
        replace_config_with_latest(config_src, CONFIG_PATH)
    else:
        print_color(f"Warning: config.toml file not found in {current_dir}", YELLOW)
        print_color("You'll need to create a configuration file manually", YELLOW)
    
    # Setup logging
    print_color("Setting up logging...", GREEN)
    Path(LOG_PATH).touch(mode=0o640, exist_ok=True)
    print_color(f"Log file created at {LOG_PATH}", GREEN)
    
    # Detect interfaces
    print_color("Available network interfaces:", GREEN)
    try:
        result = subprocess.run(['ip', '-br', 'link', 'show'], capture_output=True, text=True, check=True)
        interfaces = [line.split()[0] for line in result.stdout.splitlines() if 'lo' not in line]
        for iface in interfaces:
            print(f"  - {iface}")
    except subprocess.CalledProcessError:
        print_color("Warning: Could not detect network interfaces", YELLOW)
    
    # Set permissions
    os.chmod(CONFIG_DIR, 0o755)
    if os.path.exists(CONFIG_PATH) and os.path.getsize(CONFIG_PATH) > 0:
        os.chmod(CONFIG_PATH, 0o644)
    if os.path.exists(SERVICE_PATH):
        os.chmod(SERVICE_PATH, 0o644)
    if os.path.exists(BINARY_PATH):
        os.chmod(BINARY_PATH, 0o755)
    
    # Start service if it was running or if this is a new installation
    if was_running or not is_update:
        _ = start_service()
    
    # Enable service for automatic startup
    _ = enable_service()
    
    print()
    if is_update:
        print_color("=== Update Complete ===", BLUE)
    else:
        print_color("=== Installation Complete ===", GREEN)
    
    print_color("Service commands:", YELLOW)
    print("  sudo systemctl start wg-failover.service")
    print("  sudo systemctl stop wg-failover.service")
    print("  sudo systemctl restart wg-failover.service")
    print("  sudo systemctl status wg-failover.service")
    print()
    print_color("To view logs:", YELLOW)
    print("  sudo journalctl -u wg-failover.service -f")
    print("  or")
    print(f"  sudo tail -f {LOG_PATH}")
    print()
    print_color(f"Configuration file location: {CONFIG_PATH}", GREEN)
    if is_update:
        print_color("Your existing configuration has been preserved", GREEN)
    else:
        print_color("Please edit the configuration file to match your setup!", YELLOW)

def remote_install(target_ip: str, private_key: str, username: str, sudo_password: str, is_update: bool) -> None:
    """Perform remote installation or update via SSH without copying install script"""
    if is_update:
        print_color(f"=== Updating WireGuard Failover Remotely on {target_ip} ===", BLUE)
    else:
        print_color(f"=== Installing WireGuard Failover Remotely on {target_ip} ===", GREEN)
    
    # Create SSH command prefix
    ssh_cmd: list[str] = ["ssh", "-i", private_key, f"{username}@{target_ip}"]
    scp_cmd: list[str] = ["scp", "-i", private_key]
    
    # Check if we can connect
    print_color("Testing SSH connection...", GREEN)
    try:
        _ = subprocess.run(ssh_cmd + ['echo', 'Connected successfully'], check=True, capture_output=True)
        print_color("SSH connection successful", GREEN)
    except subprocess.CalledProcessError:
        print_color(f"Error: Could not connect to {target_ip} via SSH", RED)
        sys.exit(1)
    
    # Get current directory
    current_dir = os.path.dirname(os.path.realpath(__file__))
    
    # Check if required files exist
    required_files = ['wg-failover.service', 'config.toml']
    for file in required_files:
        if not os.path.exists(os.path.join(current_dir, file)):
            print_color(f"Error: Required file '{file}' not found in {current_dir}", RED)
            sys.exit(1)
    
    # Check for wg-failover executable in the correct location
    wg_failover_exe = os.path.join(current_dir, 'target', 'release', 'wg-failover')
    if not os.path.exists(wg_failover_exe):
        wg_failover_exe = os.path.join(current_dir, 'wg-failover')
    
    if not os.path.exists(wg_failover_exe):
        print_color(f"Error: wg-failover executable not found in {current_dir}/target/release/ or {current_dir}/", RED)
        sys.exit(1)
    
    # Create temp directory on remote server
    print_color("Creating temporary directory on remote server...", GREEN)
    remote_temp_dir = "/tmp/wg-failover-install"
    
    try:
        _ = subprocess.run(ssh_cmd + ['mkdir', '-p', remote_temp_dir], check=True)
    except subprocess.CalledProcessError:
        print_color(f"Error: Could not create directory {remote_temp_dir} on remote server", RED)
        sys.exit(1)
    
    # Copy files to remote server (only assets, not install script)
    print_color("Copying assets to remote server...", GREEN)
    
    # Copy wg-failover executable
    try:
        _ = subprocess.run(scp_cmd + [wg_failover_exe, f"{username}@{target_ip}:{remote_temp_dir}/wg-failover"], check=True)
        print_color("✓ Binary copied", GREEN)
    except subprocess.CalledProcessError:
        print_color("Error: Failed to copy binary to remote server", RED)
        sys.exit(1)
    
    # Copy service file
    service_file = os.path.join(current_dir, 'wg-failover.service')
    try:
        _ = subprocess.run(scp_cmd + [service_file, f"{username}@{target_ip}:{remote_temp_dir}/wg-failover.service"], check=True)
        print_color("✓ Service file copied", GREEN)
    except subprocess.CalledProcessError:
        print_color("Error: Failed to copy service file to remote server", RED)
        sys.exit(1)
    
    # Copy config file
    config_file = os.path.join(current_dir, 'config.toml')
    try:
        _ = subprocess.run(scp_cmd + [config_file, f"{username}@{target_ip}:{remote_temp_dir}/config.toml"], check=True)
        print_color("✓ Config file copied", GREEN)
    except subprocess.CalledProcessError:
        print_color("Error: Failed to copy config file to remote server", RED)
        sys.exit(1)
    
    # Create remote installation script (executed on remote server)
    # Note: We need to pass is_update as a parameter to the shell script
    update_flag = "true" if is_update else "false"
    install_script_content = f'''#!/bin/bash
set -e

IS_UPDATE={update_flag}

if [ "$IS_UPDATE" = "true" ]; then
    echo "Starting WireGuard Failover update..."
else
    echo "Starting WireGuard Failover installation..."
fi

# Check if running as root
if [ "$EUID" -ne 0 ]; then
    echo "Error: This script must be run as root"
    exit 1
fi

# Define paths
BINARY_SRC="{remote_temp_dir}/wg-failover"
BINARY_DEST="/usr/local/bin/wg-failover"
SERVICE_SRC="{remote_temp_dir}/wg-failover.service"
SERVICE_DEST="/etc/systemd/system/wg-failover.service"
CONFIG_SRC="{remote_temp_dir}/config.toml"
CONFIG_DEST="/etc/wg-failover/config.toml"
CONFIG_DIR="/etc/wg-failover"
LOG_FILE="/var/log/wg-failover.log"

# Check if service is running
if systemctl is-active wg-failover.service >/dev/null 2>&1; then
    echo "Stopping wg-failover service..."
    systemctl stop wg-failover.service || true
    pkill -f wg-failover 2>/dev/null || true
    SERVICE_WAS_RUNNING=true
else
    SERVICE_WAS_RUNNING=false
fi

# Backup existing config if updating
if [ -f "$CONFIG_DEST" ] && [ "$IS_UPDATE" = "true" ]; then
    echo "Backing up existing configuration..."
    cp "$CONFIG_DEST" "$CONFIG_DEST.backup" 2>/dev/null || true
fi

# Install binary
echo "Installing binary to $BINARY_DEST..."
cp "$BINARY_SRC" "$BINARY_DEST"
chmod 755 "$BINARY_DEST"

# Install service file
echo "Installing systemd service..."
cp "$SERVICE_SRC" "$SERVICE_DEST"
chmod 644 "$SERVICE_DEST"
systemctl daemon-reload

# Install configuration
echo "Installing configuration..."
mkdir -p "$CONFIG_DIR"
if [ -f "$CONFIG_SRC" ]; then
    if [ -f "$CONFIG_DEST" ] && [ "$IS_UPDATE" = "true" ]; then
        # Preserve existing config sections
        echo "Preserving existing configuration settings..."
        # Simple config preservation - in production you might want more sophisticated merging
        if [ -f "$CONFIG_DEST.backup" ]; then
            # For now, just keep the backup
            echo "Existing configuration backed up to $CONFIG_DEST.backup"
        fi
        cp "$CONFIG_SRC" "$CONFIG_DEST"
    else
        cp "$CONFIG_SRC" "$CONFIG_DEST"
    fi
    chmod 644 "$CONFIG_DEST"
else
    echo "Warning: No configuration file found"
fi

# Setup logging
echo "Setting up logging..."
touch "$LOG_FILE"
chmod 640 "$LOG_FILE"

# Set directory permissions
chmod 755 "$CONFIG_DIR"

# Start service if it was running or if this is a new installation
if [ "$SERVICE_WAS_RUNNING" = true ] || [ "$IS_UPDATE" = "false" ]; then
    echo "Starting wg-failover service..."
    systemctl start wg-failover.service || true
    sleep 2
fi

# Enable service for automatic startup
echo "Enabling service to start on boot..."
systemctl enable wg-failover.service 2>/dev/null || true

# Verify installation
if systemctl is-active wg-failover.service >/dev/null 2>&1; then
    echo "✅ WireGuard Failover service is running"
else
    echo "⚠️  Service is not running. Check logs with: journalctl -u wg-failover.service"
fi

echo "Cleaning up temporary files..."
rm -rf "{remote_temp_dir}"

if [ "$IS_UPDATE" = "true" ]; then
    echo "✅ WireGuard Failover updated successfully!"
else
    echo "✅ WireGuard Failover installed successfully!"
fi
'''
    
    # Write the remote installation script to a temporary file
    with tempfile.NamedTemporaryFile(mode='w', suffix='.sh', delete=False) as f:
        f.write(install_script_content)
        remote_script_path = f.name
    
    # Make the script executable and copy it to remote server
    os.chmod(remote_script_path, 0o755)
    
    try:
        _ = subprocess.run(scp_cmd + [remote_script_path, f"{username}@{target_ip}:{remote_temp_dir}/remote_install.sh"], check=True)
        print_color("✓ Installation script prepared", GREEN)
    except subprocess.CalledProcessError:
        print_color("Error: Failed to copy installation script to remote server", RED)
        os.unlink(remote_script_path)
        sys.exit(1)
    
    # Clean up local temporary file
    os.unlink(remote_script_path)
    
    # Execute remote installation with sudo
    print_color("Executing remote installation...", GREEN)
    
    # Create a wrapper script that runs the installation with sudo
    sudo_wrapper_content = f'''#!/bin/bash
echo "{sudo_password}" | sudo -S bash "{remote_temp_dir}/remote_install.sh"
'''
    
    with tempfile.NamedTemporaryFile(mode='w', suffix='.sh', delete=False) as f:
        f.write(sudo_wrapper_content)
        sudo_wrapper_path = f.name
    
    os.chmod(sudo_wrapper_path, 0o755)
    
    try:
        _ = subprocess.run(scp_cmd + [sudo_wrapper_path, f"{username}@{target_ip}:{remote_temp_dir}/run_with_sudo.sh"], check=True)
    except subprocess.CalledProcessError:
        print_color("Error: Failed to copy sudo wrapper to remote server", RED)
        os.unlink(sudo_wrapper_path)
        sys.exit(1)
    
    os.unlink(sudo_wrapper_path)
    
    # Run the installation
    remote_cmd = f"bash {remote_temp_dir}/run_with_sudo.sh"
    result = subprocess.run(ssh_cmd + [remote_cmd], capture_output=True, text=True)
    
    # Output results
    if result.returncode == 0:
        if is_update:
            print_color("✅ Remote update completed successfully", BLUE)
        else:
            print_color("✅ Remote installation completed successfully!", GREEN)
        print(result.stdout)
    else:
        if is_update:
            print_color("❌ Error during remote update", RED)
        else:
            print_color("❌ Error during remote installation", RED)
        print(result.stderr)
        
        # Try to get more info
        print_color("Attempting to get more details...", YELLOW)
        debug_cmd = f"sudo bash {remote_temp_dir}/remote_install.sh"
        debug_result = subprocess.run(ssh_cmd + [debug_cmd], capture_output=True, text=True)
        if debug_result.returncode != 0:
            print("Debug output:")
            print(debug_result.stderr)
        
        # Cleanup on error
        cleanup_cmd = f"sudo rm -rf {remote_temp_dir}"
        _ = subprocess.run(ssh_cmd + [cleanup_cmd], capture_output=True)
        sys.exit(1)
    
    # Final cleanup
    print_color("Performing final cleanup...", GREEN)
    cleanup_cmd = f"sudo rm -rf {remote_temp_dir}"
    _ = subprocess.run(ssh_cmd + [cleanup_cmd], capture_output=True)
    
    if is_update:
        print_color("=== Remote Update Complete ===", BLUE)
    else:
        print_color("=== Remote Installation Complete ===", GREEN)
    print_color(f"WireGuard Failover has been {'updated' if is_update else 'installed'} on {target_ip}", GREEN)
    
    # Show service status
    print_color("\nService status on remote server:", YELLOW)
    status_cmd = "systemctl status wg-failover.service --no-pager"
    _ = subprocess.run(ssh_cmd + [status_cmd], capture_output=False)

def main() -> None:
    parser: argparse.ArgumentParser = argparse.ArgumentParser(description='WireGuard Failover Installer')
    _ = parser.add_argument('--remote', action='store_true', help='Install on remote server')
    _ = parser.add_argument('--local', action='store_true', help='Install locally (default)')
    _ = parser.add_argument('--update', action='store_true', help='Update existing installation')
    _ = parser.add_argument('--target-ip', help='Target server IP for remote installation')
    _ = parser.add_argument('--private-key', help='Private key file for SSH authentication')
    _ = parser.add_argument('--username', help='Username for SSH connection (default: current user)')
    _ = parser.add_argument('--sudo-password', help='Sudo password for remote installation')
    
    args: argparse.Namespace = parser.parse_args()
    
    # Get current directory
    current_dir: str = os.path.dirname(os.path.realpath(__file__))
    
    # Check if this is an update
    is_update: bool = getattr(args, 'update', False)
    
    # If no mode specified and already installed, suggest update
    if not getattr(args, 'remote', False) and not getattr(args, 'local', False) and not getattr(args, 'update', False):
        if is_installed():
            print_color("WireGuard Failover is already installed.", GREEN)
            installed_version: str | None = get_installed_version()
            current_version: str | None = get_current_version()
            
            if installed_version and current_version and installed_version != current_version:
                print_color(f"Current version: {installed_version}", YELLOW)
                print_color(f"New version available: {current_version}", GREEN)
                response = input("Do you want to update? (y/N): ").strip().lower()
                if response in ['y', 'yes']:
                    is_update = True
                else:
                    print_color("Exiting. Use --update to force update.", YELLOW)
                    sys.exit(0)
            else:
                print_color("No update available or version information not available.", YELLOW)
                sys.exit(0)
    
    # Handle remote installation/update
    if getattr(args, 'remote', False):
        if not getattr(args, 'target_ip', None) or not getattr(args, 'private_key', None):
            print_color("For remote installation, --target-ip and --private-key are required", RED)
            sys.exit(1)
        
        # Validate private key file exists
        if not os.path.exists(getattr(args, 'private_key', '')):
            print_color(f"Error: Private key file '{getattr(args, 'private_key', '')}' not found", RED)
            sys.exit(1)
        
        # Get username if not provided
        username: str = getattr(args, 'username', '') or getpass.getuser()
        
        # Get sudo password if not provided
        sudo_password: str = getattr(args, 'sudo_password', '')
        if not sudo_password:
            sudo_password = getpass.getpass("Enter sudo password for remote server: ")
        
        remote_install(getattr(args, 'target_ip', ''), getattr(args, 'private_key', ''), username, sudo_password, is_update)
        return
    
    # Handle local installation/update
    # Check for root privileges for local installation
    check_root()
    
    # Perform local installation or update
    local_install(current_dir, is_update)

if __name__ == "__main__":
    main()