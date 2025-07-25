#!/bin/bash

# WireGuard Failover Installer Script
# This script installs the WireGuard Failover utility and systemd service

set -e
set -o pipefail

# Check for root privileges
if [ "$(id -u)" -ne 0 ]; then
    echo "Error: This script must be run as root" >&2
    exit 1
fi

# Variables
BINARY_PATH="/usr/local/bin/wg-failover"
SERVICE_PATH="/etc/systemd/system/wg-failover.service"
CONFIG_DIR="/etc/wg-failover"
CONFIG_PATH="$CONFIG_DIR/config.toml"
LOG_PATH="/var/log/wg-failover.log"

# Terminal colors
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[0;33m'
NC='\033[0m' # No Color

# Function to build the binary
build_binary() {
    echo -e "${GREEN}Building wg-failover binary...${NC}"
    if ! command -v cargo &> /dev/null; then
        echo -e "${RED}Error: Rust toolchain not found. Please install Rust first.${NC}" >&2
        echo "You can install Rust by running: curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh" >&2
        exit 1
    fi

    cargo build --release
    if [ $? -ne 0 ]; then
        echo -e "${RED}Error: Failed to build the binary${NC}" >&2
        exit 1
    fi
}

# Function to install the binary
install_binary() {
    echo -e "${GREEN}Installing binary to $BINARY_PATH...${NC}"
    if [ ! -f "target/release/wg-failover" ]; then
        echo -e "${RED}Error: Binary not found. Build failed?${NC}" >&2
        exit 1
    fi
    cp target/release/wg-failover "$BINARY_PATH"
    chmod +x "$BINARY_PATH"
    echo -e "${GREEN}Binary installed successfully${NC}"
}

# Function to install the service file
install_service() {
    echo -e "${GREEN}Installing systemd service to $SERVICE_PATH...${NC}"
    if [ ! -f "wg-failover.service" ]; then
        echo -e "${RED}Error: Service file not found${NC}" >&2
        exit 1
    fi
    cp wg-failover.service "$SERVICE_PATH"
    systemctl daemon-reload
    echo -e "${GREEN}Service installed successfully${NC}"
}

# Function to install the configuration
install_config() {
    echo -e "${GREEN}Installing configuration to $CONFIG_PATH...${NC}"
    mkdir -p "$CONFIG_DIR"
    
    # Only copy the config file if it doesn't exist already
    if [ ! -f "$CONFIG_PATH" ]; then
        if [ ! -f "config.toml" ]; then
            echo -e "${RED}Error: Configuration template not found${NC}" >&2
            exit 1
        fi
        cp config.toml "$CONFIG_PATH"
        echo -e "${GREEN}Default configuration installed. Please edit $CONFIG_PATH to match your setup.${NC}"
    else
        echo -e "${YELLOW}Configuration already exists at $CONFIG_PATH. Not overwriting.${NC}"
    fi
}

# Function to create log file
setup_logging() {
    echo -e "${GREEN}Setting up logging...${NC}"
    touch "$LOG_PATH"
    chmod 640 "$LOG_PATH"
    echo -e "${GREEN}Log file created at $LOG_PATH${NC}"
}

# Function to detect network interfaces
detect_interfaces() {
    echo -e "${GREEN}Available network interfaces:${NC}"
    if ! command -v ip &> /dev/null; then
        echo -e "${YELLOW}Warning: 'ip' command not found. Cannot detect network interfaces.${NC}" >&2
        return
    fi
    
    ip -br link show | grep -v "lo" | awk '{print "  - "$1}'
    echo ""
    echo -e "${YELLOW}Please edit $CONFIG_PATH and set the appropriate interfaces.${NC}"
}

# Main installation function
install() {
    echo -e "${GREEN}=== Installing WireGuard Failover ===${NC}"
    
    # Check if we're in the right directory
    if [ ! -f "Cargo.toml" ] || ! grep -q "wg-failover" "Cargo.toml"; then
        echo -e "${RED}Error: Please run this script from the project root directory${NC}" >&2
        exit 1
    fi
    
    # Check for dependencies
    echo -e "${GREEN}Checking dependencies...${NC}"
    for cmd in cargo ip systemctl; do
        if ! command -v $cmd &> /dev/null; then
            echo -e "${RED}Error: Required command '$cmd' not found${NC}" >&2
            echo "Please install the required packages before continuing." >&2
            exit 1
        fi
    done
    
    build_binary
    install_binary
    install_service
    install_config
    setup_logging
    detect_interfaces
    
    echo ""
    echo -e "${GREEN}=== Installation Complete ===${NC}"
    echo -e "${YELLOW}To start the service, run:${NC}"
    echo "  sudo systemctl start wg-failover.service"
    echo ""
    echo -e "${YELLOW}To enable automatic startup on boot:${NC}"
    echo "  sudo systemctl enable wg-failover.service"
    echo ""
    echo -e "${YELLOW}To check service status:${NC}"
    echo "  sudo systemctl status wg-failover.service"
    echo ""
    echo -e "${YELLOW}To view logs:${NC}"
    echo "  sudo journalctl -u wg-failover.service -f"
    echo "  or"
    echo "  sudo tail -f $LOG_PATH"
    
    echo ""
    echo -e "${GREEN}Configuration file location: $CONFIG_PATH${NC}"
    echo -e "${RED}You MUST edit this file before starting the service!${NC}"
    echo -e "${GREEN}Set your network interfaces, WireGuard interface, and peer IP in this file${NC}"
}

# Check if user is root
if [ "$(id -u)" -ne 0 ]; then
    echo -e "${RED}Error: This script must be run as root${NC}" >&2
    echo "Please run with: sudo $0" >&2
    exit 1
fi

# Run the installation
install