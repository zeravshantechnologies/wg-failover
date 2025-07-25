# WireGuard Network Failover

A Rust application for monitoring and automatically switching between multiple network interfaces to maintain continuous WireGuard VPN connectivity. This ensures your VPN connection remains active even when one of your network interfaces (such as Ethernet or WiFi) fails.

## Overview

This tool solves the problem of maintaining uninterrupted VPN connectivity when a host has multiple network interfaces (e.g., Ethernet and WiFi). When one network connection fails, the application automatically switches the default route to the working interface and reconfigures the WireGuard tunnel.

## Features

- Automatic detection of network interface failures
- Seamless switching between primary and secondary interfaces
- Support for both NetworkManager and wg-quick setups
- Non-persistent routing changes (temporary, not persisted through reboots)
- Command-line interface with configurable parameters
- Detailed logging for monitoring and troubleshooting
- Systemd service integration
- Configuration via config file or command-line parameters
- WiFi signal strength monitoring
- TCP connection testing as an alternative to ping

## Installation

### Prerequisites

- Rust toolchain (1.60 or newer)
- Linux operating system
- NetworkManager or wg-quick for WireGuard management
- Root permissions for network changes

### Building from Source

```bash
# Clone the repository
git clone https://github.com/yourusername/wg-failover.git
cd wg-failover

# Build the project
cargo build --release

# Install the binary and associated files
sudo ./install.sh

# Or manually install just the binary
sudo cp target/release/wg-failover /usr/local/bin/
```

### Systemd Service Setup

The installation script will set up the systemd service automatically. If you need to do it manually:

```bash
# Copy the service file
sudo cp wg-failover.service /etc/systemd/system/

# Edit the service file to use your interfaces
sudo nano /etc/systemd/system/wg-failover.service

# Reload systemd
sudo systemctl daemon-reload

# Important: The service requires a configuration file
# Create a config file if you haven't already
sudo nano /etc/wg-failover/config.toml

# Start the service (no need for @parameter when using config file)
sudo systemctl start wg-failover.service

# Enable the service to start on boot
sudo systemctl enable wg-failover.service
```

## Usage

### Command Line

```bash
# Basic usage
sudo wg-failover --peer-ip 192.168.1.1 --primary eth0 --secondary wlan0

# Full options
# Example using config file (preferred method)
sudo wg-failover

# Example using command line parameters (temporary testing only)
sudo wg-failover \
  --peer-ip 192.168.1.1 \
  --primary eth0 \
  --secondary wlan0 \
  --interval 30 \
  --count 2 \
  --timeout 2
```

### Configuration File

You must create a configuration file at `/etc/wg-failover/config.toml` before starting the service:

```toml
# WireGuard peer to monitor
[peer]
ip = "10.0.0.1"
count = 2
timeout = 2

# WireGuard interface settings
[wireguard]
interface = "wg0"
restart_method = "nmcli"

# Network interfaces
[interfaces]
primary = "enp0s31f6"
secondary = "wlp0s20f0u5"

# Monitoring settings
[monitoring]
interval = 30
log_level = "info"
```

### Configuration Options

You can configure wg-failover either through the config file or command line parameters. For persistent setups, the config file is recommended.

**Config File Options (/etc/wg-failover/config.toml):**

- `--peer-ip, -i`: IP address or hostname of the WireGuard peer
- `--primary, -p`: Primary network interface
- `--secondary, -s`: Secondary (fallback) network interface
- `--interval, -t`: Check interval in seconds (default: 30)
- `--count, -c`: Number of ping attempts (default: 2)
- `--timeout, -w`: Ping timeout in seconds (default: 2)

## Important Notes

1. **Configuration is required**: You must create `/etc/wg-failover/config.toml` before starting the service
2. **Interface names**: Use `ip link show` to find your actual interface names
3. **WireGuard interface**: Should match your WireGuard configuration
4. **Service doesn't use @parameter**: When using the config file, start with `wg-failover.service` without @parameter

## How It Works

1. The application continuously monitors connectivity to the specified peer IP
2. When the primary interface loses connectivity, it:
   - Removes the current default route
   - Adds a new default route via the secondary interface
   - Restarts the WireGuard connection
3. When the primary interface regains connectivity, it automatically switches back
4. All changes are temporary and won't persist after a reboot

### Technical Details

- Uses `ip route` commands to modify the routing table
- Leverages both NetworkManager and wg-quick for managing WireGuard connections
- Monitors network status using ping or TCP connection tests
- For wireless interfaces, can monitor signal strength
- Handles graceful termination via SIGINT (Ctrl+C)

## Troubleshooting

### Common Issues

- **Permission denied**: Make sure to run with `sudo` or as root
- **Interface not found**: Verify interface names with `ip link show`
- **WireGuard restart fails**: Check if the WireGuard interface is managed by NetworkManager or wg-quick
- **No connectivity after switch**: Verify that the gateway was correctly detected

### Logging

Enable debug logging by setting the environment variable:

```bash
sudo RUST_LOG=debug wg-failover --peer-ip 192.168.1.1 --wg-interface wg0 --primary eth0 --secondary wlan0
```

## License

MIT

## Contributing

Contributions are welcome! Please feel free to submit a Pull Request.

1. Fork the repository
2. Create your feature branch (`git checkout -b feature/amazing-feature`)
3. Commit your changes (`git commit -m 'Add some amazing feature'`)
4. Push to the branch (`git push origin feature/amazing-feature`)
5. Open a Pull Request
