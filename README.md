# WireGuard Network Failover with Multiple IP Testing

A Rust application for monitoring and automatically switching between multiple network interfaces to maintain continuous WireGuard VPN connectivity. This ensures your VPN connection remains active even when one of your network interfaces fails, providing maximum uptime for web services and applications.

**New Features:**
- **Multiple IP Testing**: Test connectivity to multiple IP addresses for better network health assessment
- **Full Traffic Routing**: Option to route all traffic through selected interface, not just WireGuard peer traffic

## Overview

This tool solves the problem of maintaining uninterrupted VPN connectivity when a host has multiple network interfaces (e.g., Ethernet and WiFi). When one network connection fails, the application automatically switches the default route to the working interface and reconfigures the WireGuard tunnel.

## Features

- **Automatic failover**: Immediate switching between primary and secondary network interfaces when connectivity is lost
- **Speed-based optimization**: Periodically tests interface speeds and switches to faster connections
- **Configurable thresholds**: Set minimum speed improvement percentage before switching
- **Anti-flapping protection**: Minimum time between switches to prevent rapid toggling
- **Dual monitoring**: Quick connectivity checks + periodic speed tests
- **Multiple IP testing**: Test connectivity to multiple IPs for accurate network health assessment
- **Flexible routing**: Route all traffic or just WireGuard peer traffic through selected interface
- **Flexible configuration**: Command-line arguments or configuration file
- **Detailed logging**: Comprehensive monitoring and troubleshooting with failover counters
- **Systemd service integration**: Easy deployment as a system service

## Installation

### Prerequisites

- Rust toolchain (1.60 or newer) for building from source
- Python 3.6+ for the installation script (no external dependencies required)
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
```

### Automated Installation

The project includes a Python installation script that can install locally or remotely:

#### Local Installation

```bash
# No external Python dependencies required - using only standard library modules

# Note: For remote installation, the target server must have Python 3 and pip installed

# Run the installation script
sudo python3 install.py --local
```

#### Remote Installation

```bash
# Install Python dependencies
pip install -r requirements.txt

# Run the remote installation (will prompt for sudo password)
python3 install.py --remote \
  --target-ip 192.168.1.100 \
  --private-key ~/.ssh/id_rsa \
  --username your_username

# Or provide sudo password via command line
python3 install.py --remote \
  --target-ip 192.168.1.100 \
  --private-key ~/.ssh/id_rsa \
  --username your_username \
  --sudo-password your_sudo_password
```

### Manual Installation

If you prefer manual installation:

```bash
# Copy the binary
sudo cp target/release/wg-failover /usr/local/bin/

# Copy the service file
sudo cp wg-failover.service /etc/systemd/system/

# Create configuration directory
sudo mkdir -p /etc/wg-failover

# Create configuration file
sudo cp config.toml /etc/wg-failover/config.toml

# Edit the configuration file with your settings
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
  --primary eth0 \
  --secondary wlan0

# Full options with speed testing
sudo wg-failover \
  --peer-ip 192.168.1.1 \
  --primary eth0 \
  --secondary wlan0 \
  --interval 30 \
  --speedtest-interval 3600 \
  --speed-threshold 35

# Using configuration file
sudo wg-failover --config /etc/wg-failover/config.toml

# Debug logging
sudo RUST_LOG=debug wg-failover --config /etc/wg-failover/config.toml

# New: Multiple IP testing
sudo wg-failover \
  --peer-ip 192.168.1.1 \
  --primary eth0 \
  --secondary wlan0 \
  --test-ips "8.8.8.8,1.1.1.1,192.168.1.1" \
  --interval 30

# New: Route all traffic through selected interface
sudo wg-failover \
  --peer-ip 192.168.1.1 \
  --primary eth0 \
  --secondary wlan0 \
  --test-ips "8.8.8.8,1.1.1.1" \
  --route-all-traffic \
  --interval 30
```

### Configuration File

Create `/etc/wg-failover/config.toml`:

```toml
# WireGuard peer to monitor
[peer]
# Public IP address or hostname of the WireGuard peer to ping
ip = "206.189.140.174"

# Network interfaces
[interfaces]
# Primary network interface (preferred)
primary = "enp2s0f0u2"
# Secondary network interface (fallback)
secondary = "enp10s0"

# Test IPs for connectivity checks
# List of IP addresses to test for connectivity
# At least 50% of tests must succeed for interface to be considered working
test_ips = [
    "8.8.8.8",        # Google DNS
    "1.1.1.1",        # Cloudflare DNS
    "208.67.222.222", # OpenDNS
    "206.189.140.174", # WireGuard peer (included automatically)
]

# Route all traffic through selected interface
# When true: All traffic (0.0.0.0/0) will be routed through selected interface
# When false: Only traffic to WireGuard peer will be routed through selected interface
route_all_traffic = false

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
- `-p, --primary <PRIMARY>`: Primary network interface (e.g., eth0)
- `-s, --secondary <SECONDARY>`: Secondary network interface (e.g., wlan0)
- `-t, --interval <INTERVAL>`: Connectivity check interval in seconds [default: 30]
- `--speedtest-interval <SPEEDTEST_INTERVAL>`: Speed test interval in seconds [default: 3600]
- `--speed-threshold <SPEED_THRESHOLD>`: Speed threshold percentage to switch to faster interface [default: 35]
- `--test-ips <TEST_IPS>`: Comma-separated list of IPs to test for connectivity [default: 8.8.8.8,1.1.1.1,208.67.222.222,peer-ip]
- `--route-all-traffic`: Route all traffic through selected interface, not just WireGuard peer traffic

## How It Works

### Enhanced Monitoring System

1. **Multiple IP Connectivity Monitoring (Fast)**
   - Tests connectivity to multiple IP addresses (configurable list)
   - Interface considered working if at least 50% of tests succeed
   - Immediately switches to backup interface if primary fails
   - Automatically switches back to primary when it recovers
   - Provides more accurate network health assessment than single IP testing

2. **Speed Optimization (Periodic)**
   - Performs speed tests every hour (configurable)
   - Compares latency between interfaces using detailed ping tests
   - Switches to faster interface if it's at least configured percentage faster
   - Always prefers primary interface unless secondary is significantly faster

3. **Flexible Routing Options**
   - **Peer-only routing**: Only route traffic to WireGuard peer through selected interface
   - **Full traffic routing**: Route all system traffic (0.0.0.0/0) through selected interface
   - Configurable via command line or configuration file

### Operation Modes

- **Automatic Failover Mode**: When primary interface loses connectivity, immediately switch to secondary
- **Speed Optimization Mode**: When both interfaces are active, use the faster one
- **Auto-recovery**: Automatically switch back to primary when it becomes available
- **Anti-flapping**: Minimum time between switches to prevent rapid toggling
- **Multiple IP Testing**: Test connectivity to multiple IPs for accurate network assessment
- **Flexible Routing**: Choose between peer-only or full traffic routing

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
- **Rapid interface switching**: The anti-flapping protection prevents rapid switching
- **Route-all-traffic changes default route**: Be cautious when enabling `route_all_traffic` as it changes system's default route
- **Multiple IP tests all failing**: Check if test IPs are reachable from your network

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

# Test interface connectivity to multiple IPs
ping -I eth0 8.8.8.8
ping -I eth0 1.1.1.1
ping -I wlan0 8.8.8.8
ping -I wlan0 1.1.1.1

# Check specific routes
ip route get 8.8.8.8
ip route get 1.1.1.1

# Check default route
ip route show default
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