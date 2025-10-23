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

def check_root():
    """Check if running as root"""
    if os.geteuid() != 0:
        print_color("Error: This script must be run as root", RED)
        sys.exit(1)

def local_install(current_dir):
    """Perform local installation"""
    print_color("=== Installing WireGuard Failover Locally ===", GREEN)
    
    # Check for required commands
    print_color("Checking dependencies...", GREEN)
    required_commands = ['ip', 'systemctl']
    for cmd in required_commands:
        if not shutil.which(cmd):
            print_color(f"Error: Required command '{cmd}' not found", RED)
            print_color("Please install the required packages before continuing.", RED)
            sys.exit(1)
    
    # Install binary
    # First check in target/release directory (standard Rust build location)
    binary_src = os.path.join(current_dir, 'target', 'release', 'wg-failover')
    if not os.path.exists(binary_src):
        # Fallback to current directory
        binary_src = os.path.join(current_dir, 'wg-failover')
    
    if not os.path.exists(binary_src):
        print_color(f"Error: wg-failover executable not found in {current_dir}/target/release/ or {current_dir}/", RED)
        sys.exit(1)
    
    print_color(f"Installing binary to {BINARY_PATH}...", GREEN)
    shutil.copy(binary_src, BINARY_PATH)
    os.chmod(BINARY_PATH, 0o755)
    print_color("Binary installed successfully", GREEN)
    
    # Install service file
    service_src = os.path.join(current_dir, 'wg-failover.service')
    if not os.path.exists(service_src):
        print_color(f"Error: wg-failover.service file not found in {current_dir}", RED)
        sys.exit(1)
    
    print_color(f"Installing systemd service to {SERVICE_PATH}...", GREEN)
    shutil.copy(service_src, SERVICE_PATH)
    subprocess.run(['systemctl', 'daemon-reload'], check=True)
    print_color("Service installed successfully", GREEN)
    
    # Install configuration
    print_color(f"Installing configuration to {CONFIG_PATH}...", GREEN)
    os.makedirs(CONFIG_DIR, mode=0o755, exist_ok=True)
    
    config_src = os.path.join(current_dir, 'config.toml')
    if not os.path.exists(config_src):
        print_color(f"Error: config.toml file not found in {current_dir}", RED)
        sys.exit(1)
    
    # Copy config as-is
    shutil.copy(config_src, CONFIG_PATH)
    print_color("Configuration installed successfully", GREEN)
    
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
    os.chmod(CONFIG_PATH, 0o644)
    
    print()
    print_color("=== Installation Complete ===", GREEN)
    print_color("To start the service, run:", YELLOW)
    print("  sudo systemctl start wg-failover.service")
    print()
    print_color("To enable automatic startup on boot:", YELLOW)
    print("  sudo systemctl enable wg-failover.service")
    print()
    print_color("To check service status:", YELLOW)
    print("  sudo systemctl status wg-failover.service")
    print()
    print_color("To view logs:", YELLOW)
    print("  sudo journalctl -u wg-failover.service -f")
    print("  or")
    print(f"  sudo tail -f {LOG_PATH}")
    print()
    print_color(f"Configuration file location: {CONFIG_PATH}", GREEN)
    print_color("Please edit this file to match your setup before starting the service!", YELLOW)

def remote_install(target_ip, private_key, username, sudo_password):
    """Perform remote installation via SSH with sudo"""
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
    
    # Copy files
    for file in required_files:
        local_file = os.path.join(current_dir, file)
        subprocess.run(scp_cmd + [local_file, f"{username}@{target_ip}:{remote_temp_dir}/"], check=True)
    
    # Copy wg-failover executable
    subprocess.run(scp_cmd + [wg_failover_exe, f"{username}@{target_ip}:{remote_temp_dir}/"], check=True)
    
    # Copy install script
    install_script = os.path.join(current_dir, 'install.py')
    subprocess.run(scp_cmd + [install_script, f"{username}@{target_ip}:{remote_temp_dir}/"], check=True)
    
    # Create a script to run the installation with sudo
    sudo_script_content = f'''#!/bin/bash
# Run the installation
echo "{sudo_password}" | sudo -S python3 {remote_temp_dir}/install.py --local
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
        print_color("Remote installation completed successfully", GREEN)
        print(result.stdout)
    else:
        print_color("Error during remote installation", RED)
        print(result.stderr)
        # Try to get more detailed error info
        debug_cmd = f"cd {remote_temp_dir} && echo '{sudo_password}' | sudo -S python3 {remote_temp_dir}/install.py --local"
        debug_result = subprocess.run(ssh_cmd + [debug_cmd], capture_output=True, text=True)
        if debug_result.returncode != 0:
            print("Debug output:")
            print(debug_result.stderr)
        sys.exit(1)
    
    # Cleanup temporary files
    print_color("Cleaning up temporary files...", GREEN)
    subprocess.run(ssh_cmd + ['rm', '-rf', remote_temp_dir], check=True)
    
    print_color("=== Remote Installation Complete ===", GREEN)
    print_color(f"WireGuard Failover has been installed on {target_ip}", GREEN)
    print_color("You can now connect to the remote server to start and configure the service", YELLOW)

def main():
    parser = argparse.ArgumentParser(description='WireGuard Failover Installer')
    parser.add_argument('--remote', action='store_true', help='Install on remote server')
    parser.add_argument('--local', action='store_true', help='Install locally (default)')
    parser.add_argument('--target-ip', help='Target server IP for remote installation')
    parser.add_argument('--private-key', help='Private key file for SSH authentication')
    parser.add_argument('--username', help='Username for SSH connection (default: current user)')
    parser.add_argument('--sudo-password', help='Sudo password for remote installation')
    
    args = parser.parse_args()
    
    # Get current directory
    current_dir = os.path.dirname(os.path.realpath(__file__))
    
    # Handle remote installation
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
        
        remote_install(args.target_ip, args.private_key, username, sudo_password)
        return
    
    # Handle local installation
    # Check for root privileges for local installation
    check_root()
    
    # Perform local installation
    local_install(current_dir)

if __name__ == "__main__":
    main()