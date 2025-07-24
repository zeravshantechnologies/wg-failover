mod network;
mod errors;

use anyhow::{Context, Result};
use chrono::Local;
use clap::Parser;
use log::{error, info, debug, warn};
use network::{get_current_interface, ping_interface, switch_interface, interface_exists,
               get_interface_addresses, is_wireless_interface, get_wifi_signal_strength, 
               tcp_connection_test, list_interfaces};
use std::{fs, path::Path, thread, time};
use std::process::exit;

/// WireGuard Failover - A utility for ensuring continuous VPN connectivity
/// by monitoring multiple network interfaces and switching between them when necessary.
#[derive(Parser, Debug)]
#[clap(author, version, about)]
struct Args {
    /// The IP address or hostname of the WireGuard peer
    #[clap(short, long)]
    peer_ip: String,

    /// The WireGuard interface name (e.g., wg0)
    #[clap(short, long)]
    wg_interface: String,

    /// Primary network interface (e.g., eth0, enp0s31f6)
    #[clap(short, long)]
    primary: String,

    /// Secondary network interface (e.g., wlan0, wlp0s20f0u5)
    #[clap(short, long)]
    secondary: String,

    /// Ping interval in seconds
    #[clap(short, long, default_value = "30")]
    interval: u64,

    /// Number of ping attempts
    #[clap(short, long, default_value = "2")]
    count: u8,

    /// Ping timeout in seconds
    #[clap(short = 'w', long, default_value = "2")]
    timeout: u8,
    
    /// Use TCP connection test instead of ping (port 443)
    #[clap(short = 't', long)]
    use_tcp: bool,
}

#[derive(Debug, Clone, PartialEq)]
enum NetworkStatus {
    Primary,
    Secondary,
    Unavailable,
}

fn log_with_timestamp(msg: &str) {
    info!("[{}] {}", Local::now().format("%Y-%m-%d %H:%M:%S"), msg);
}

// Using network module functions now

/// Attempts to load a configuration file
fn load_config_file(config_path: &str) -> Result<Option<Args>> {
    let path = Path::new(config_path);
    if !path.exists() {
        debug!("Config file not found at {}", config_path);
        return Ok(None);
    }
    
    debug!("Loading config from {}", config_path);
    let config_str = fs::read_to_string(path)
        .context(format!("Failed to read config file at {}", config_path))?;
    
    let config: toml::Value = toml::from_str(&config_str)
        .context("Failed to parse TOML config")?;
    
    // Extract values with defaults
    let peer_ip = config
        .get("peer")
        .and_then(|p| p.get("ip"))
        .and_then(|ip| ip.as_str())
        .unwrap_or("")
        .to_string();
    
    let wg_interface = config
        .get("wireguard")
        .and_then(|w| w.get("interface"))
        .and_then(|i| i.as_str())
        .unwrap_or("wg0")
        .to_string();
    
    let primary = config
        .get("interfaces")
        .and_then(|i| i.get("primary"))
        .and_then(|p| p.as_str())
        .unwrap_or("")
        .to_string();
    
    let secondary = config
        .get("interfaces")
        .and_then(|i| i.get("secondary"))
        .and_then(|s| s.as_str())
        .unwrap_or("")
        .to_string();
    
    let interval = config
        .get("monitoring")
        .and_then(|m| m.get("interval"))
        .and_then(|i| i.as_integer())
        .unwrap_or(30) as u64;
    
    let count = config
        .get("peer")
        .and_then(|p| p.get("count"))
        .and_then(|c| c.as_integer())
        .unwrap_or(2) as u8;
    
    let timeout = config
        .get("peer")
        .and_then(|p| p.get("timeout"))
        .and_then(|t| t.as_integer())
        .unwrap_or(2) as u8;
        
    let use_tcp = config
        .get("monitoring")
        .and_then(|m| m.get("use_tcp"))
        .and_then(|t| t.as_bool())
        .unwrap_or(false);
    
    // Validate required fields
    if peer_ip.is_empty() || primary.is_empty() || secondary.is_empty() {
        debug!("Config file missing required fields (peer IP, primary or secondary interface)");
        return Ok(None);
    }
    
    Ok(Some(Args {
        peer_ip,
        wg_interface,
        primary,
        secondary,
        interval,
        count,
        timeout,
        use_tcp,
    }))
}

fn main() -> Result<()> {
    // Initialize logging with environment variables
    env_logger::init();
    
    // Try to load config file first
    let config_paths = [
        "/etc/wg-failover/config.toml",
        "./config.toml",
    ];
    
    let mut args = None;
    for path in &config_paths {
        if let Ok(Some(config_args)) = load_config_file(path) {
            info!("Loaded configuration from {}", path);
            args = Some(config_args);
            break;
        }
    }
    
    // If no config file was found or it was invalid, use command line args
    let args = args.unwrap_or_else(Args::parse);
    
    info!("WireGuard Failover Monitor started");
    info!("Configuration:");
    info!("  Peer IP: {}", args.peer_ip);
    info!("  WireGuard Interface: {}", args.wg_interface);
    info!("  Primary Interface: {}", args.primary);
    info!("  Secondary Interface: {}", args.secondary);
    info!("  Check Interval: {} seconds", args.interval);
    info!("  Ping Count: {}", args.count);
    info!("  Ping Timeout: {} seconds", args.timeout);
    info!("  Using TCP Test: {}", args.use_tcp);
    
    // List all available interfaces
    let available_interfaces = list_interfaces();
    info!("Available network interfaces:");
    for iface in &available_interfaces {
        info!("  - {}", iface);
    }
    
    // Verify selected interfaces are in the list
    if !available_interfaces.contains(&args.primary) {
        warn!("Selected primary interface '{}' may not exist", args.primary);
    }
    
    if !available_interfaces.contains(&args.secondary) {
        warn!("Selected secondary interface '{}' may not exist", args.secondary);
    }
    
    // Check if running as root
    #[cfg(unix)]
    if unsafe { libc::geteuid() } != 0 {
        error!("This program must be run as root. Please use sudo or run as the root user.");
        exit(1);
    }
    
    // Check if interfaces exist and show information
    if !interface_exists(&args.primary) {
        return Err(anyhow::anyhow!(
            "Primary interface '{}' not found",
            args.primary
        ));
    } else {
        info!("Primary interface {} found", args.primary);
        if let Some(addrs) = get_interface_addresses(&args.primary).first() {
            info!("Primary interface IP: {}", addrs);
        }
        
        if is_wireless_interface(&args.primary) {
            info!("Primary interface is wireless");
            if let Some(signal) = get_wifi_signal_strength(&args.primary) {
                info!("Primary interface signal strength: {} dBm", signal);
            }
        }
    }
    
    if !interface_exists(&args.secondary) {
        return Err(anyhow::anyhow!(
            "Secondary interface '{}' not found",
            args.secondary
        ));
    } else {
        info!("Secondary interface {} found", args.secondary);
        if let Some(addrs) = get_interface_addresses(&args.secondary).first() {
            info!("Secondary interface IP: {}", addrs);
        }
        
        if is_wireless_interface(&args.secondary) {
            info!("Secondary interface is wireless");
            if let Some(signal) = get_wifi_signal_strength(&args.secondary) {
                info!("Secondary interface signal strength: {} dBm", signal);
            }
        }
    }
    
    // Handle Ctrl+C gracefully
    ctrlc::set_handler(move || {
        info!("Received termination signal. Exiting...");
        // We could do cleanup here if needed
        exit(0);
    })?;
    
    let mut last_status = NetworkStatus::Unavailable;
    
    // Initial status report
    info!("Starting network monitoring loop");
    info!("Press Ctrl+C to exit");
    
    loop {
        // Check wireless signal strength periodically
        if is_wireless_interface(&args.primary) {
            if let Some(signal) = get_wifi_signal_strength(&args.primary) {
                debug!("Primary wireless signal strength: {} dBm", signal);
                if signal < -80 {
                    warn!("Primary interface has weak signal: {} dBm", signal);
                }
            }
        }
        
        if is_wireless_interface(&args.secondary) {
            if let Some(signal) = get_wifi_signal_strength(&args.secondary) {
                debug!("Secondary wireless signal strength: {} dBm", signal);
                if signal < -80 {
                    warn!("Secondary interface has weak signal: {} dBm", signal);
                }
            }
        }
    
        // Use either ping or TCP test depending on configuration
        let (primary_ok, secondary_ok) = if args.use_tcp {
            // Use TCP connection test with port 443 (HTTPS)
            let primary_ok = tcp_connection_test(&args.peer_ip, 443, args.timeout);
            let secondary_ok = tcp_connection_test(&args.peer_ip, 443, args.timeout);
            debug!("TCP connection test results: primary={}, secondary={}", primary_ok, secondary_ok);
            (primary_ok, secondary_ok)
        } else {
            // Use traditional ping
            let primary_ok = ping_interface(&args.primary, &args.peer_ip, args.count, args.timeout);
            let secondary_ok = ping_interface(&args.secondary, &args.peer_ip, args.count, args.timeout);
            debug!("Ping test results: primary={}, secondary={}", primary_ok, secondary_ok);
            (primary_ok, secondary_ok)
        };
        
        let current_iface = get_current_interface(&args.peer_ip);
        
        let current_status = if primary_ok {
            NetworkStatus::Primary
        } else if secondary_ok {
            NetworkStatus::Secondary
        } else {
            NetworkStatus::Unavailable
        };
        
        match (current_status, last_status.clone()) {
            (NetworkStatus::Primary, status) if status != NetworkStatus::Primary => {
                log_with_timestamp("✅ Primary interface is up. Switching back.");
                if let Err(e) = switch_interface(&args.primary, &args.wg_interface) {
                    error!("Failed to switch to primary interface: {}", e);
                } else {
                    last_status = NetworkStatus::Primary;
                }
            }
            (NetworkStatus::Primary, NetworkStatus::Primary) => {
                if current_iface.as_deref() != Some(&args.primary) {
                    log_with_timestamp("⚠️ Routing inconsistency detected. Fixing primary route.");
                    if let Err(e) = switch_interface(&args.primary, &args.wg_interface) {
                        error!("Failed to fix primary route: {}", e);
                    }
                } else {
                    log_with_timestamp("✅ Primary interface is active and working correctly.");
                }
            }
            (NetworkStatus::Secondary, status) if status != NetworkStatus::Secondary => {
                log_with_timestamp("⚠️ Primary is down. Switching to secondary interface.");
                if let Err(e) = switch_interface(&args.secondary, &args.wg_interface) {
                    error!("Failed to switch to secondary interface: {}", e);
                } else {
                    last_status = NetworkStatus::Secondary;
                }
            }
            (NetworkStatus::Secondary, NetworkStatus::Secondary) => {
                if current_iface.as_deref() != Some(&args.secondary) {
                    log_with_timestamp("⚠️ Routing inconsistency detected. Fixing secondary route.");
                    if let Err(e) = switch_interface(&args.secondary, &args.wg_interface) {
                        error!("Failed to fix secondary route: {}", e);
                    }
                } else {
                    log_with_timestamp("✅ Secondary interface is active and working correctly.");
                }
            }
            (NetworkStatus::Unavailable, _) => {
                log_with_timestamp("❌ Both interfaces are unreachable. WireGuard connectivity lost.");
                last_status = NetworkStatus::Unavailable;
            }
            _ => {}
        }

        thread::sleep(time::Duration::from_secs(args.interval));
    }
}