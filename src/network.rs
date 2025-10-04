use anyhow::{Context, Result};
use log::debug;
use std::process::Command;

/// Check if the given interface can reach the peer via ping
pub fn ping_interface(iface: &str, peer_ip: &str, count: u8, timeout: u8) -> bool {
    debug!("Pinging {} from interface {}", peer_ip, iface);
    
    let output = Command::new("ping")
        .args([
            "-I", iface,
            "-c", &count.to_string(),
            "-W", &timeout.to_string(),
            peer_ip,
        ])
        .output();

    match output {
        Ok(o) => o.status.success(),
        Err(_) => false
    }
}

/// Get the gateway for a specific network interface
pub fn get_gateway_for_interface(iface: &str) -> Option<String> {
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

/// Get the current interface being used to reach the peer
pub fn get_current_interface(peer_ip: &str) -> Option<String> {
    debug!("Checking current interface used to reach {}", peer_ip);
    
    let output = Command::new("ip")
        .args(["route", "get", peer_ip])
        .output()
        .ok()?;

    let route = String::from_utf8_lossy(&output.stdout);
    debug!("Raw route output for {}: {}", peer_ip, route);
    
    for part in route.split_whitespace() {
        if part == "dev" {
            let interface = route
                .split_whitespace()
                .skip_while(|&x| x != "dev")
                .nth(1)
                .map(|s| s.to_string());
            debug!("Found interface for {}: {:?}", peer_ip, interface);
            return interface;
        }
    }

    debug!("No interface found for {}", peer_ip);
    None
}

/// Switch the route to the peer to use the specified interface
pub fn switch_interface(iface: &str, peer_ip: &str) -> Result<()> {
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
pub fn interface_exists(iface: &str) -> bool {
    Command::new("ip")
        .args(["link", "show", "dev", iface])
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

/// Get available network interfaces
pub fn list_interfaces() -> Vec<String> {
    let output = Command::new("ip")
        .args(["link", "show"])
        .output();
        
    match output {
        Ok(o) => {
            let stdout = String::from_utf8_lossy(&o.stdout);
            let mut interfaces = Vec::new();
            
            for line in stdout.lines() {
                if line.contains(": ") && !line.contains("@") {
                    if let Some(iface_with_num) = line.split(": ").next() {
                        if let Some(iface_name) = iface_with_num.split_whitespace().nth(1) {
                            interfaces.push(iface_name.to_string());
                        }
                    }
                }
            }
            
            interfaces
        },
        Err(_) => Vec::new()
    }
}