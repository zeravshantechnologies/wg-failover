use anyhow::{Context, Result};
use chrono::Local;
use clap::Parser;
use log::{error, info, debug, warn};
use serde::Deserialize;
use std::{thread, time};
use std::process::exit;
use std::process::Command;
use std::env;

/// WireGuard Failover - A utility for ensuring continuous VPN connectivity
/// by managing routes to the WireGuard peer
#[derive(Parser, Debug)]
#[clap(author, version, about)]
struct Args {
    /// The IP address or hostname of the WireGuard peer
    #[clap(short = 'i', long)]
    peer_ip: Option<String>,

    /// Primary network interface (e.g., eth0, enp0s31f6)
    #[clap(short = 'p', long)]
    primary: Option<String>,

    /// Secondary network interface (e.g., wlan0, wlp0s20f0u5)
    #[clap(short = 's', long)]
    secondary: Option<String>,

    /// Ping interval in seconds
    #[clap(short = 't', long, default_value = "30")]
    interval: u64,

    /// Number of ping attempts
    #[clap(short, long, default_value = "2")]
    count: u8,

    /// Ping timeout in seconds
    #[clap(short = 'w', long, default_value = "2")]
    timeout: u8,
}

/// Configuration file structure
#[derive(Debug, Deserialize)]
struct Config {
    #[serde(rename = "peer")]
    peer_config: PeerConfig,
    #[serde(rename = "interfaces")]
    interface_config: InterfaceConfig,
    #[serde(rename = "monitoring")]
    monitoring_config: MonitoringConfig,
}

#[derive(Debug, Deserialize)]
struct PeerConfig {
    ip: String,
    count: Option<u8>,
    timeout: Option<u8>,
}

#[derive(Debug, Deserialize)]
struct InterfaceConfig {
    primary: String,
    secondary: String,
}

#[derive(Debug, Deserialize)]
struct MonitoringConfig {
    interval: Option<u64>,
}

fn log_with_timestamp(msg: &str) {
    info!("[{}] {}", Local::now().format("%Y-%m-%d %H:%M:%S"), msg);
}

/// Check if the given interface can reach the peer via ping
fn ping_interface(iface: &str, peer_ip: &str, count: u8, timeout: u8) -> bool {
    debug!("Pinging {} from interface {}", peer_ip, iface);
    
    Command::new("ping")
        .args([
            "-I", iface,
            "-c", &count.to_string(),
            "-W", &timeout.to_string(),
            peer_ip,
        ])
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

/// Get the current interface being used to reach the peer
fn get_current_interface(peer_ip: &str) -> Option<String> {
    debug!("Checking current interface used to reach {}", peer_ip);
    
    let output = Command::new("ip")
        .args(["route", "get", peer_ip])
        .output()
        .ok()?;

    let route = String::from_utf8_lossy(&output.stdout);
    for part in route.split_whitespace() {
        if part == "dev" {
            return route
                .split_whitespace()
                .skip_while(|&x| x != "dev")
                .nth(1)
                .map(|s| s.to_string());
        }
    }
    None
}

/// Get the gateway for a specific network interface
fn get_gateway_for_interface(iface: &str) -> Option<String> {
    debug!("Looking for gateway for interface {}", iface);
    
    let output = Command::new("ip")
        .args(["route", "show", "default"])
        .output()
        .ok()?;

    let routes = String::from_utf8_lossy(&output.stdout);
    for line in routes.lines() {
        if line.contains(iface) {
            let parts: Vec<&str> = line.split_whitespace().collect();
            if let Some(index) = parts.iter().position(|&x| x == "via") {
                return parts.get(index + 1).map(|s| s.to_string());
            }
        }
    }
    None
}

/// Switch the route to the peer to use the specified interface
fn switch_interface(iface: &str, peer_ip: &str) -> Result<()> {
    let gateway = get_gateway_for_interface(iface)
        .context(format!("Failed to find gateway for {}", iface))?;
    
    debug!("Switching route for {} to interface {} via {}", peer_ip, iface, gateway);

    // Delete existing route to the peer
    let _ = Command::new("ip")
        .args(["route", "del", peer_ip])
        .output();
    
    // Add new route to the peer
    Command::new("ip")
        .args(["route", "add", peer_ip, "via", &gateway, "dev", iface])
        .output()
        .context("Failed to add route to peer")?;
        
    debug!("Successfully switched route for {} to interface {}", peer_ip, iface);
    Ok(())
}

/// Check if a given interface exists
fn interface_exists(iface: &str) -> bool {
    Command::new("ip")
        .args(["link", "show", "dev", iface])
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

fn main() -> Result<()> {
    // Initialize logging
    env_logger::init();
    
    // Parse command line arguments
    let args = Args::parse();
    
    // Load configuration from environment or CLI
    let config = match env::var("WG_FAILOVER_CONFIG") {
        Ok(path) => {
            info!("Loading configuration from: {}", path);
            let config_str = std::fs::read_to_string(&path)
                .context(format!("Failed to read config file: {}", path))?;
            toml::from_str::<Config>(&config_str)
                .context("Failed to parse config file")?
        }
        Err(_) => {
            info!("Using CLI arguments");
            Config {
                peer_config: PeerConfig {
                    ip: args.peer_ip.context("--peer-ip required when no config file is specified")?,
                    count: Some(args.count),
                    timeout: Some(args.timeout),
                },
                interface_config: InterfaceConfig {
                    primary: args.primary.context("--primary required when no config file is specified")?,
                    secondary: args.secondary.context("--secondary required when no config file is specified")?,
                },
                monitoring_config: MonitoringConfig {
                    interval: Some(args.interval),
                },
            }
        }
    };
    
    // Extract parameters from config
    let peer_ip = config.peer_config.ip;
    let primary = config.interface_config.primary;
    let secondary = config.interface_config.secondary;
    let interval = config.monitoring_config.interval.unwrap_or(args.interval);
    let count = config.peer_config.count.unwrap_or(args.count);
    let timeout = config.peer_config.timeout.unwrap_or(args.timeout);
    
    info!("WireGuard Failover started");
    info!("Configuration:");
    info!("  Peer IP: {}", peer_ip);
    info!("  Primary Interface: {}", primary);
    info!("  Secondary Interface: {}", secondary);
    info!("  Check Interval: {} seconds", interval);
    
    // Verify interfaces exist
    if !interface_exists(&primary) {
        return Err(anyhow::anyhow!(
            "Primary interface '{}' not found",
            primary
        ));
    }
    
    if !interface_exists(&secondary) {
        return Err(anyhow::anyhow!(
            "Secondary interface '{}' not found",
            secondary
        ));
    }
    
    // Handle Ctrl+C gracefully
    ctrlc::set_handler(move || {
        info!("Received termination signal. Exiting...");
        exit(0);
    })?;
    
    // Main monitoring loop
    loop {
        // Log interface status
        info!("Checking primary interface: {}", primary);
        let primary_ok = ping_interface(&primary, &peer_ip, count, timeout);
        info!("Primary interface {} connectivity to {}: {}",
            primary, peer_ip, if primary_ok { "OK" } else { "FAIL" });
        
        info!("Checking secondary interface: {}", secondary);
        let secondary_ok = ping_interface(&secondary, &peer_ip, count, timeout);
        info!("Secondary interface {} connectivity to {}: {}",
            secondary, peer_ip, if secondary_ok { "OK" } else { "FAIL" });
        
        // Get current route interface
        let current_iface = get_current_interface(&peer_ip);
        info!("Current route to {} is via interface: {:?}",
            peer_ip, current_iface.as_deref().unwrap_or("unknown"));
        
        match (primary_ok, secondary_ok) {
            (true, _) => {
                // Primary is up - use it
                if current_iface.as_deref() != Some(&primary) {
                    log_with_timestamp("✅ Primary interface is up. Switching route.");
                    info!("Switching route for {} to primary interface {}", peer_ip, primary);
                    if let Err(e) = switch_interface(&primary, &peer_ip) {
                        error!("Failed to switch to primary interface: {}", e);
                    } else {
                        info!("Successfully switched to primary interface");
                        if let Some(new_iface) = get_current_interface(&peer_ip) {
                            info!("Route to {} now via: {}", peer_ip, new_iface);
                        }
                    }
                } else {
                    log_with_timestamp("✅ Primary interface is active and working correctly.");
                    info!("Traffic to {} already routed through primary interface", peer_ip);
                }
            }
            (false, true) => {
                // Primary is down, secondary is up - use secondary
                if current_iface.as_deref() != Some(&secondary) {
                    log_with_timestamp("⚠️ Primary is down. Switching to secondary interface.");
                    info!("Switching route for {} to secondary interface {}", peer_ip, secondary);
                    if let Err(e) = switch_interface(&secondary, &peer_ip) {
                        error!("Failed to switch to secondary interface: {}", e);
                    } else {
                        info!("Successfully switched to secondary interface");
                        if let Some(new_iface) = get_current_interface(&peer_ip) {
                            info!("Route to {} now via: {}", peer_ip, new_iface);
                        }
                    }
                } else {
                    log_with_timestamp("✅ Secondary interface is active and working correctly.");
                    info!("Traffic to {} already routed through secondary interface", peer_ip);
                }
            }
            (false, false) => {
                log_with_timestamp("❌ Both interfaces are unreachable. Trying to reconnect...");
                // Try to re-establish connection using last known good interface
                info!("Trying to reconnect using last known good interface");
                if let Some(iface) = current_iface.clone() {
                    info!("Attempting reconnect via interface: {}", iface);
                    if let Err(e) = switch_interface(&iface, &peer_ip) {
                        error!("Failed to reconnect: {}", e);
                    } else {
                        info!("Reconnect attempt completed");
                        if let Some(new_iface) = get_current_interface(&peer_ip) {
                            info!("Current route to {}: {}", peer_ip, new_iface);
                        }
                    }
                } else {
                    warn!("No known good interface available for reconnection");
                }
            }
        }

        thread::sleep(time::Duration::from_secs(interval));
    }
}