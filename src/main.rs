use anyhow::{Context, Result};
use chrono::Local;
use clap::Parser;
use log::{error, info, debug, warn};
use std::{thread, time};
use std::process::exit;
use std::process::Command;

/// WireGuard Failover - A utility for ensuring continuous VPN connectivity
/// by managing routes to the WireGuard peer
#[derive(Parser, Debug)]
#[clap(author, version, about)]
struct Args {
    /// The IP address or hostname of the WireGuard peer
    #[clap(short = 'i', long)]
    peer_ip: String,

    /// Primary network interface (e.g., eth0, enp0s31f6)
    #[clap(short = 'p', long)]
    primary: String,

    /// Secondary network interface (e.g., wlan0, wlp0s20f0u5)
    #[clap(short = 's', long)]
    secondary: String,

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
    
    info!("WireGuard Failover started");
    info!("Configuration:");
    info!("  Peer IP: {}", args.peer_ip);
    info!("  Primary Interface: {}", args.primary);
    info!("  Secondary Interface: {}", args.secondary);
    info!("  Check Interval: {} seconds", args.interval);
    
    // Verify interfaces exist
    if !interface_exists(&args.primary) {
        return Err(anyhow::anyhow!(
            "Primary interface '{}' not found",
            args.primary
        ));
    }
    
    if !interface_exists(&args.secondary) {
        return Err(anyhow::anyhow!(
            "Secondary interface '{}' not found",
            args.secondary
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
        info!("Checking primary interface: {}", args.primary);
        let primary_ok = ping_interface(&args.primary, &args.peer_ip, args.count, args.timeout);
        info!("Primary interface {} connectivity to {}: {}",
            args.primary, args.peer_ip, if primary_ok { "OK" } else { "FAIL" });
        
        info!("Checking secondary interface: {}", args.secondary);
        let secondary_ok = ping_interface(&args.secondary, &args.peer_ip, args.count, args.timeout);
        info!("Secondary interface {} connectivity to {}: {}",
            args.secondary, args.peer_ip, if secondary_ok { "OK" } else { "FAIL" });
        
        // Get current route interface
        let current_iface = get_current_interface(&args.peer_ip);
        info!("Current route to {} is via interface: {:?}",
            args.peer_ip, current_iface.as_deref().unwrap_or("unknown"));
        
        match (primary_ok, secondary_ok) {
            (true, _) => {
                // Primary is up - use it
                if current_iface.as_deref() != Some(&args.primary) {
                    log_with_timestamp("✅ Primary interface is up. Switching route.");
                    info!("Switching route for {} to primary interface {}", args.peer_ip, args.primary);
                    if let Err(e) = switch_interface(&args.primary, &args.peer_ip) {
                        error!("Failed to switch to primary interface: {}", e);
                    } else {
                        info!("Successfully switched to primary interface");
                        if let Some(new_iface) = get_current_interface(&args.peer_ip) {
                            info!("Route to {} now via: {}", args.peer_ip, new_iface);
                        }
                    }
                } else {
                    log_with_timestamp("✅ Primary interface is active and working correctly.");
                    info!("Traffic to {} already routed through primary interface", args.peer_ip);
                }
            }
            (false, true) => {
                // Primary is down, secondary is up - use secondary
                if current_iface.as_deref() != Some(&args.secondary) {
                    log_with_timestamp("⚠️ Primary is down. Switching to secondary interface.");
                    info!("Switching route for {} to secondary interface {}", args.peer_ip, args.secondary);
                    if let Err(e) = switch_interface(&args.secondary, &args.peer_ip) {
                        error!("Failed to switch to secondary interface: {}", e);
                    } else {
                        info!("Successfully switched to secondary interface");
                        if let Some(new_iface) = get_current_interface(&args.peer_ip) {
                            info!("Route to {} now via: {}", args.peer_ip, new_iface);
                        }
                    }
                } else {
                    log_with_timestamp("✅ Secondary interface is active and working correctly.");
                    info!("Traffic to {} already routed through secondary interface", args.peer_ip);
                }
            }
            (false, false) => {
                log_with_timestamp("❌ Both interfaces are unreachable. Trying to reconnect...");
                // Try to re-establish connection using last known good interface
                info!("Trying to reconnect using last known good interface");
                if let Some(iface) = current_iface.clone() {
                    info!("Attempting reconnect via interface: {}", iface);
                    if let Err(e) = switch_interface(&iface, &args.peer_ip) {
                        error!("Failed to reconnect: {}", e);
                    } else {
                        info!("Reconnect attempt completed");
                        if let Some(new_iface) = get_current_interface(&args.peer_ip) {
                            info!("Current route to {}: {}", args.peer_ip, new_iface);
                        }
                    }
                } else {
                    warn!("No known good interface available for reconnection");
                }
            }
        }

        thread::sleep(time::Duration::from_secs(args.interval));
    }
}