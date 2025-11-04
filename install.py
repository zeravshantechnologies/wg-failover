#!/usr/bin/env python3

import os
import sys
import subprocess
import argparse
import getpass
import shutil
import tempfile
from pathlib import Path


# Terminal colors
RED = '\033[0;31m'
GREEN = '\033[0;32m'
YELLOW = '\033[0;33m'
BLUE = '\033[0;34m'
NC = '\033[0m'  # No Color

# Default paths
BINARY_PATH = "/usr/local/bin/wg-failover"
SERVICE_PATH = "/etc/systemd/system/wg-failover.service"
CONFIG_DIR = "/etc/wg-failover"
CONFIG_PATH = f"{CONFIG_DIR}/config.toml"
LOG_PATH = "/var/log/wg-failover.log"

def print_color(message, color=NC):
    """Print colored output"""
    print(f"{color}{message}{NC}")

def replace_config_with_latest(new_config_path, existing_config_path):
    """Replace existing config file with latest version"""
    try:
        # Create backup of existing config
        backup_path = f"{existing_config_path}.backup"
        if os.path.exists(existing_config_path):
            shutil.copy2(existing_config_path, backup_path)
            print_color(f"Backed up existing config to: {backup_path}", YELLOW)
        
        # Replace with new config
        shutil.copy2(new_config_path, existing_config_path)
        os.chmod(existing_config_path, 0o644)
        print_color("Configuration replaced with latest version", GREEN)
        
    except Exception as e:
        print_color(f"Error replacing configuration: {e}", RED)
        # Try to restore from backup if replacement failed
        if os.path.exists(backup_path):
            shutil.copy2(backup_path, existing_config_path)
            print_color("Restored configuration from backup", YELLOW)

def check_root():
    """Check if running as root"""
    if os.geteuid() != 0:
        print_color("Error: This script must be run as root", RED)
        sys.exit(1)

def is_installed():
    """Check if wg-failover is already installed"""
    return os.path.exists(BINARY_PATH) and os.path.exists(SERVICE_PATH)

def is_service_running():
    """Check if the wg-failover service is currently running"""
    try:
        result = subprocess.run(['systemctl', 'is-active', 'wg-failover.service'], 
                              capture_output=True, text=True)
        return result.returncode == 0
    except Exception:
        return False

def get_installed_version():
    """Get version of installed wg-failover"""
    if not is_installed():
        return None
    
    try:
        result = subprocess.run([BINARY_PATH, "--version"], capture_output=True, text=True, check=True)
        version_line = result.stdout.strip()
        # Extract version number from output like "wg-failover 0.1.0"
        if " " in version_line:
            return version_line.split(" ")[1]
        return version_line
    except (subprocess.CalledProcessError, FileNotFoundError):
        return None

def get_current_version():
    """Get version of current wg-failover binary"""
    current_dir = os.path.dirname(os.path.realpath(__file__))
    
    # Check for binary in target/release first
    binary_src = os.path.join(current_dir, 'target', 'release', 'wg-failover')
    if not os.path.exists(binary_src):
        # Fallback to current directory
        binary_src = os.path.join(current_dir, 'wg-failover')
    
    if not os.path.exists(binary_src):
        return None
    
    try:
        result = subprocess.run([binary_src, "--version"], capture_output=True, text=True, check=True)
        version_line = result.stdout.strip()
        if " " in version_line:
            return version_line.split(" ")[1]
        return version_line
    except (subprocess.CalledProcessError, FileNotFoundError):
        return None

def backup_config():
    """Backup existing configuration"""
    if os.path.exists(CONFIG_PATH):
        backup_path = f"{CONFIG_PATH}.backup"
        print_color(f"Backing up existing configuration to {backup_path}...", YELLOW)
        shutil.copy2(CONFIG_PATH, backup_path)
        print_color("Configuration backup created", GREEN)
        return backup_path
    return None

def restore_config():
    """Restore configuration from backup"""
    backup_path = f"{CONFIG_PATH}.backup"
    if os.path.exists(backup_path):
        print_color("Restoring configuration from backup...", YELLOW)
        shutil.copy2(backup_path, CONFIG_PATH)
        os.remove(backup_path)
        print_color("Configuration restored", GREEN)

def stop_service():
    """Stop the wg-failover service"""
    print_color("Stopping wg-failover service...", YELLOW)
    try:
        result = subprocess.run(['systemctl', 'stop', 'wg-failover.service'], capture_output=True, text=True)
        if result.returncode == 0:
            print_color("Service stopped", GREEN)
            return True
        else:
            print_color(f"Service stop failed: {result.stderr}", YELLOW)
            # Force kill any remaining processes
            subprocess.run(['pkill', '-f', 'wg-failover'], capture_output=True)
            return False
    except Exception as e:
        print_color(f"Error stopping service: {e}", YELLOW)
        return False

def start_service():
    """Start the wg-failover service"""
    print_color("Starting wg-failover service...", YELLOW)
    try:
        result = subprocess.run(['systemctl', 'start', 'wg-failover.service'], capture_output=True, text=True)
        if result.returncode == 0:
            print_color("Service started", GREEN)
            # Wait a moment and verify it's actually running
            import time
            time.sleep(2)
            if is_service_running():
                print_color("Service verified as running", GREEN)
                return True
            else:
                print_color("Warning: Service started but may not be running", YELLOW)
                return False
        else:
            print_color(f"Service start failed: {result.stderr}", RED)
            return False
    except Exception as e:
        print_color(f"Error starting service: {e}", RED)
        return False

def enable_service():
    """Enable the wg-failover service to start on boot"""
    print_color("Enabling wg-failover service...", YELLOW)
    try:
        result = subprocess.run(['systemctl', 'enable', 'wg-failover.service'], capture_output=True, text=True)
        if result.returncode == 0:
            print_color("Service enabled for automatic startup", GREEN)
            return True
        else:
            print_color(f"Warning: Could not enable service: {result.stderr}", YELLOW)
            return False
    except Exception as e:
        print_color(f"Warning: Could not enable service: {e}", YELLOW)
        return False

def local_install(current_dir, is_update=False):
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
        backup_config()
        
        # Stop service before update
        was_running = stop_service()
    else:
        print_color("=== Installing WireGuard Failover Locally ===", GREEN)
        was_running = False
    
    # Check for required commands
    print_color("Checking dependencies...", GREEN)
    required_commands = ['ip', 'systemctl', 'nmcli']
    missing_commands = []
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
    shutil.copy(binary_src, BINARY_PATH)
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
    shutil.copy(service_src, SERVICE_PATH)
    os.chmod(SERVICE_PATH, 0o644)
    subprocess.run(['systemctl', 'daemon-reload'], check=True)
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
    if os.path.exists(CONFIG_PATH):
        os.chmod(CONFIG_PATH, 0o644)
    if os.path.exists(SERVICE_PATH):
        os.chmod(SERVICE_PATH, 0o644)
    if os.path.exists(BINARY_PATH):
        os.chmod(BINARY_PATH, 0o755)
    
    # Start service if it was running or if this is a new installation
    if was_running or not is_update:
        start_service()
    
    # Enable service for automatic startup
    enable_service()
    
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

def remote_install(target_ip, private_key, username, sudo_password, is_update=False):
    """Perform remote installation or update via SSH with sudo"""
    if is_update:
        print_color(f"=== Updating WireGuard Failover Remotely on {target_ip} ===", BLUE)
    else:
        print_color(f"=== Installing WireGuard Failover Remotely on {target_ip} ===", GREEN)
    
    # Create SSH command prefix
    ssh_cmd = ['ssh', '-i', private_key, f"{username}@{target_ip}"]
    scp_cmd = ['scp', '-i', private_key]
    
    # Check if we can connect
    print_color("Testing SSH connection...", GREEN)
    try:
        subprocess.run(ssh_cmd + ['echo', 'Connected successfully'], check=True, capture_output=True)
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
    
    # Copy files to remote server
    print_color("Copying files to remote server...", GREEN)
    remote_temp_dir = "/tmp/wg-failover-install"
    
    # Create temp directory on remote server
    subprocess.run(ssh_cmd + ['mkdir', '-p', remote_temp_dir], check=True)
    
    # Copy all required files to remote server
    for file in required_files:
        local_file = os.path.join(current_dir, file)
        subprocess.run(scp_cmd + [local_file, f"{username}@{target_ip}:{remote_temp_dir}/"], check=True)
    
    # Copy wg-failover executable
    subprocess.run(scp_cmd + [wg_failover_exe, f"{username}@{target_ip}:{remote_temp_dir}/"], check=True)
    
    # Copy install script
    install_script = os.path.join(current_dir, 'install.py')
    subprocess.run(scp_cmd + [install_script, f"{username}@{target_ip}:{remote_temp_dir}/"], check=True)
    
    # Create installation script
    install_mode = "--update" if is_update else "--local"
    sudo_script_content = f'''#!/bin/bash
# Run the installation/update
echo "{sudo_password}" | sudo -S python3 {remote_temp_dir}/install.py {install_mode}

# Force restart service to ensure new binary is running
sudo systemctl stop wg-failover.service 2>/dev/null || true
sudo pkill -f wg-failover 2>/dev/null || true
sleep 2
sudo systemctl start wg-failover.service 2>/dev/null || true
sleep 3

# Verify service is running
if systemctl is-active wg-failover.service >/dev/null 2>&1; then
    echo "✅ Service successfully restarted with new binary"
else
    echo "❌ Service restart failed - check logs"
    # Try one more time with force restart
    sudo systemctl restart wg-failover.service 2>/dev/null || true
fi
'''
    
    # Write the sudo script to a temporary file
    with tempfile.NamedTemporaryFile(mode='w', suffix='.sh', delete=False) as f:
        f.write(sudo_script_content)
        sudo_script_path = f.name
    
    # Make the script executable and copy it to remote server
    os.chmod(sudo_script_path, 0o755)
    subprocess.run(scp_cmd + [sudo_script_path, f"{username}@{target_ip}:{remote_temp_dir}/install_with_sudo.sh"], check=True)
    os.unlink(sudo_script_path)  # Clean up temporary file
    
    # Run installation on remote server with sudo
    print_color("Running installation on remote server...", GREEN)
    remote_cmd = f"cd {remote_temp_dir} && bash install_with_sudo.sh"
    result = subprocess.run(ssh_cmd + [remote_cmd], capture_output=True, text=True)
    
    if result.returncode == 0:
        if is_update:
            print_color("Remote update completed successfully", BLUE)
        else:
            print_color("Remote installation completed successfully", GREEN)
        print(result.stdout)
    else:
        if is_update:
            print_color("Error during remote update", RED)
        else:
            print_color("Error during remote installation", RED)
        print(result.stderr)
        # Try to get more detailed error info
        debug_cmd = f"cd {remote_temp_dir} && echo '{sudo_password}' | sudo -S python3 {remote_temp_dir}/install.py {install_mode}"
        debug_result = subprocess.run(ssh_cmd + [debug_cmd], capture_output=True, text=True)
        if debug_result.returncode != 0:
            print("Debug output:")
            print(debug_result.stderr)
        sys.exit(1)
    
    # Cleanup temporary files
    print_color("Cleaning up temporary files...", GREEN)
    subprocess.run(ssh_cmd + ['rm', '-rf', remote_temp_dir], check=True)
    
    if is_update:
        print_color("=== Remote Update Complete ===", BLUE)
    else:
        print_color("=== Remote Installation Complete ===", GREEN)
    print_color(f"WireGuard Failover has been {'updated' if is_update else 'installed'} on {target_ip}", GREEN)

def main():
    parser = argparse.ArgumentParser(description='WireGuard Failover Installer')
    parser.add_argument('--remote', action='store_true', help='Install on remote server')
    parser.add_argument('--local', action='store_true', help='Install locally (default)')
    parser.add_argument('--update', action='store_true', help='Update existing installation')
    parser.add_argument('--target-ip', help='Target server IP for remote installation')
    parser.add_argument('--private-key', help='Private key file for SSH authentication')
    parser.add_argument('--username', help='Username for SSH connection (default: current user)')
    parser.add_argument('--sudo-password', help='Sudo password for remote installation')
    
    args = parser.parse_args()
    
    # Get current directory
    current_dir = os.path.dirname(os.path.realpath(__file__))
    
    # Check if this is an update
    is_update = args.update
    
    # If no mode specified and already installed, suggest update
    if not args.remote and not args.local and not args.update:
        if is_installed():
            print_color("WireGuard Failover is already installed.", YELLOW)
            installed_version = get_installed_version()
            current_version = get_current_version()
            
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
    if args.remote:
        if not args.target_ip or not args.private_key:
            print_color("For remote installation, --target-ip and --private-key are required", RED)
            sys.exit(1)
        
        # Validate private key file exists
        if not os.path.exists(args.private_key):
            print_color(f"Error: Private key file '{args.private_key}' not found", RED)
            sys.exit(1)
        
        # Get username if not provided
        username = args.username if args.username else getpass.getuser()
        
        # Get sudo password if not provided
        sudo_password = args.sudo_password
        if not sudo_password:
            sudo_password = getpass.getpass("Enter sudo password for remote server: ")
        
        remote_install(args.target_ip, args.private_key, username, sudo_password, is_update)
        return
    
    # Handle local installation/update
    # Check for root privileges for local installation
    check_root()
    
    # Perform local installation or update
    local_install(current_dir, is_update)

if __name__ == "__main__":
    main()