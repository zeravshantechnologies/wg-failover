# WireGuard Network Failover

A Rust application for monitoring and automatically switching between multiple network interfaces to maintain continuous WireGuard VPN connectivity. This ensures your VPN connection remains active even when one of your network interfaces (such as Ethernet or WiFi) fails.

## Overview

This tool solves the problem of maintaining uninterrupted VPN connectivity when a host has multiple network interfaces (e.g., Ethernet and WiFi). When one network connection fails, the application automatically switches the default route to the working interface and reconfigures the WireGuard tunnel.

## Features

- **Automatic failover**: Seamless switching between primary and secondary network interfaces
- **Speed-based optimization**: Periodically tests interface speeds and switches to faster connections
- **Configurable thresholds**: Set minimum speed improvement percentage before switching
- **Dual monitoring**: Quick connectivity checks + periodic speed tests
- **Flexible configuration**: Command-line arguments or configuration file
- **Detailed logging**: Comprehensive monitoring and troubleshooting
- **Systemd service integration**: Easy deployment as a system service

## Installation

### Prerequisites

- Rust toolchain (1.60 or newer)
- Linux operating system
- `speedtest-cli` package for speed testing functionality
- Root permissions for network changes

### Building from Source

```bash
# Clone the repository
git clone https://github.com/yourusername/wg-failover.git
cd wg-failover

# Build the project
cargo build --release

# Install the binary
sudo cp target/release/wg-failover /usr/local/bin/
```

### Systemd Service Setup

```bash
# Copy the service file
sudo cp wg-failover.service /etc/systemd/system/

# Create configuration directory
sudo mkdir -p /etc/wg-failover

# Create configuration file
sudo nano /etc/wg-failover/config.toml

# Reload systemd
sudo systemctl daemon-reload

# Start the service
sudo systemctl start wg-failover.service

# Enable the service to start on boot
sudo systemctl enable wg-failover.service
```

## Usage

### Command Line Interface

```bash
# Basic usage with CLI arguments
sudo wg-failover \
  --peer-ip 192.168.1.1 \
  --wg-interface wg0 \
  --primary eth0 \
  --secondary wlan0

# Full options with speed testing
sudo wg-failover \
  --peer-ip 192.168.1.1 \
  --wg-interface wg0 \
  --primary eth0 \
  --secondary wlan0 \
  --interval 30 \
  --count 2 \
  --timeout 2 \
  --speedtest-interval 3600 \
  --speed-threshold 35

# Using configuration file
sudo wg-failover --config /etc/wg-failover/config.toml

# Debug logging
sudo RUST_LOG=debug wg-failover --config /etc/wg-failover/config.toml
```

### Configuration File

Create `/etc/wg-failover/config.toml`:

```toml
# WireGuard peer to monitor
[peer]
# Public IP address or hostname of the WireGuard peer to ping
ip = "206.189.140.174"
# Number of ping attempts
count = 2
# Ping timeout in seconds
timeout = 2

# WireGuard interface settings
[wireguard]
# Name of the WireGuard interface
interface = "wg0"

# Network interfaces
[interfaces]
# Primary network interface (preferred)
primary = "enp2s0f0u2"
# Secondary network interface (fallback)
secondary = "enp10s0"

# Monitoring settings
[monitoring]
# Connectivity check interval in seconds
interval = 30
# Speed test interval in seconds (default: 3600 = 1 hour)
speedtest_interval = 3600
# Speed threshold percentage to switch to faster interface (default: 35)
speed_threshold = 35
# Log level: "error", "warn", "info", "debug", "trace"
log_level = "info"
```

### Command Line Options

- `--config <CONFIG>`: Path to configuration file
- `-i, --peer-ip <PEER_IP>`: IP address or hostname of the WireGuard peer
- `-w, --wg-interface <WG_INTERFACE>`: WireGuard interface name (e.g., wg0)
- `-p, --primary <PRIMARY>`: Primary network interface (e.g., eth0)
- `-s, --secondary <SECONDARY>`: Secondary network interface (e.g., wlan0)
- `-t, --interval <INTERVAL>`: Connectivity check interval in seconds [default: 30]
- `-n, --count <COUNT>`: Number of ping attempts [default: 2]
- `--timeout <TIMEOUT>`: Ping timeout in seconds [default: 2]
- `--speedtest-interval <SPEEDTEST_INTERVAL>`: Speed test interval in seconds [default: 3600]
- `--speed-threshold <SPEED_THRESHOLD>`: Speed threshold percentage to switch to faster interface [default: 35]

## How It Works

### Dual Monitoring System

1. **Connectivity Monitoring (Fast)**
   - Checks interface connectivity every 30 seconds (configurable)
   - Immediately switches to backup interface if primary fails
   - Ensures continuous VPN connectivity

2. **Speed Optimization (Periodic)**
   - Performs speed tests every hour (configurable)
   - Compares download speeds between interfaces
   - Switches to faster interface if it's at least 35% faster (configurable)
   - Always prefers primary interface unless secondary is significantly faster

### Operation Modes

- **Failover Mode**: When primary interface loses connectivity, immediately switch to secondary
- **Speed Optimization Mode**: When both interfaces are active, use the faster one
- **Auto-recovery**: Automatically switch back to primary when it becomes available and faster

## Configuration Priority

1. Command-line arguments (highest priority)
2. Configuration file specified with `--config`
3. Environment variable `WG_FAILOVER_CONFIG`
4. Default configuration file locations

## Troubleshooting

### Common Issues

- **Permission denied**: Ensure you're running with `sudo` or as root
- **Interface not found**: Verify interface names with `ip link show`
- **Speed test fails**: Install `speedtest-cli` package
- **No connectivity after switch**: Check gateway detection and routing tables

### Debug Mode

Enable detailed logging for troubleshooting:

```bash
sudo RUST_LOG=debug wg-failover --config /etc/wg-failover/config.toml
```

### Interface Verification

```bash
# List available interfaces
ip link show

# Check current routing
ip route show

# Test interface connectivity
ping -I eth0 8.8.8.8
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
```

Now let me check if the code compiles with all the changes: