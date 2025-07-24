use anyhow::{Context, Result};
use log::{debug, warn};
use std::process::Command;
use std::str;
use std::net::TcpStream;
use std::time::Duration;

/// Check if the given interface can reach the peer via ping
pub fn ping_interface(iface: &str, peer_ip: &str, count: u8, timeout: u8) -> bool {
    debug!("Pinging {} from interface {}", peer_ip, iface);
    
    let output = Command::new("ping")
        .args([
            "-I",
            iface,
            "-c",
            &count.to_string(),
            "-W",
            &timeout.to_string(),
            peer_ip,
        ])
        .output();

    match output {
        Ok(o) => {
            let success = o.status.success();
            if success {
                debug!("Ping successful on interface {}", iface);
            } else {
                debug!("Ping failed on interface {}: {}", iface, 
                    str::from_utf8(&o.stderr).unwrap_or("Unknown error"));
            }
            success
        },
        Err(e) => {
            warn!("Failed to execute ping on {}: {}", iface, e);
            false
        }
    }
}

/// Get the default gateway for a specific network interface
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
                let gateway = parts.get(index + 1).map(|s| s.to_string());
                debug!("Found gateway {} for interface {}", gateway.as_ref().unwrap_or(&"<none>".to_string()), iface);
                return gateway;
            }
        }
    }

    debug!("No gateway found for interface {}", iface);
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
    debug!("Route lookup result: {}", route);
    
    for part in route.split_whitespace() {
        if part == "dev" {
            let iface = route
                .split_whitespace()
                .skip_while(|&x| x != "dev")
                .nth(1)
                .map(|s| s.to_string());
                
            if let Some(ref if_name) = iface {
                debug!("Current interface to reach {} is {}", peer_ip, if_name);
            } else {
                debug!("Could not determine current interface");
            }
            
            return iface;
        }
    }

    debug!("No route found to reach {}", peer_ip);
    None
}

/// Switch the default route to use the specified interface
pub fn switch_interface(iface: &str, wg_interface: &str) -> Result<()> {
    let gateway = get_gateway_for_interface(iface)
        .context(format!("Failed to find gateway for {}", iface))?;
    
    debug!("Switching default route to interface {} via {}", iface, gateway);

    // Delete current default route
    let del_result = Command::new("ip")
        .args(["route", "del", "default"])
        .output()
        .context("Failed to delete default route")?;
    
    if !del_result.status.success() {
        debug!(
            "Note: Could not delete default route: {}",
            String::from_utf8_lossy(&del_result.stderr)
        );
        // Continue anyway, as the route might not exist
    }
    
    // Add new default route
    let add_result = Command::new("ip")
        .args(["route", "add", "default", "via", &gateway, "dev", iface])
        .output()
        .context("Failed to add new default route")?;
        
    if !add_result.status.success() {
        return Err(anyhow::anyhow!(
            "Failed to add route: {}",
            String::from_utf8_lossy(&add_result.stderr)
        ));
    }

    // Restart WireGuard interface
    debug!("Restarting WireGuard interface {}", wg_interface);
    
    // Try NetworkManager first
    let wg_down = Command::new("nmcli")
        .args(["con", "down", wg_interface])
        .output();
        
    match wg_down {
        Ok(output) => {
            if !output.status.success() {
                warn!(
                    "Failed to bring down WireGuard with nmcli: {}",
                    String::from_utf8_lossy(&output.stderr)
                );
                // Fallback to wg-quick if nmcli fails
                fallback_wg_restart(wg_interface)?;
            } else {
                // If nmcli down worked, try nmcli up
                let wg_up = Command::new("nmcli")
                    .args(["con", "up", wg_interface])
                    .output()
                    .context("Failed to bring up WireGuard interface")?;
                    
                if !wg_up.status.success() {
                    return Err(anyhow::anyhow!(
                        "Failed to bring up WireGuard: {}",
                        String::from_utf8_lossy(&wg_up.stderr)
                    ));
                }
            }
        },
        Err(_) => {
            // nmcli not available, try wg-quick instead
            fallback_wg_restart(wg_interface)?;
        }
    }
    
    debug!("Successfully switched to interface {}", iface);
    Ok(())
}

/// Fallback to wg-quick if nmcli is not available
fn fallback_wg_restart(wg_interface: &str) -> Result<()> {
    debug!("Falling back to wg-quick for interface {}", wg_interface);
    
    // Try wg-quick down
    let wg_down = Command::new("wg-quick")
        .args(["down", wg_interface])
        .output()
        .context("Failed to execute wg-quick down")?;
        
    if !wg_down.status.success() {
        warn!(
            "Failed to bring down WireGuard with wg-quick: {}",
            String::from_utf8_lossy(&wg_down.stderr)
        );
    }
    
    // Try wg-quick up
    let wg_up = Command::new("wg-quick")
        .args(["up", wg_interface])
        .output()
        .context("Failed to execute wg-quick up")?;
        
    if !wg_up.status.success() {
        return Err(anyhow::anyhow!(
            "Failed to bring up WireGuard with wg-quick: {}",
            String::from_utf8_lossy(&wg_up.stderr)
        ));
    }
    
    Ok(())
}

/// Check if a given interface exists
pub fn interface_exists(iface: &str) -> bool {
    let output = Command::new("ip")
        .args(["link", "show", "dev", iface])
        .output();
        
    match output {
        Ok(o) => o.status.success(),
        Err(_) => false
    }
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
            
            // Parse output to extract interface names
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

/// Checks if a TCP connection can be established to a host:port
/// This provides an alternative to ping for checking connectivity
pub fn tcp_connection_test(host: &str, port: u16, timeout_secs: u8) -> bool {
    debug!("Testing TCP connection to {}:{}", host, port);

    let timeout = Duration::from_secs(timeout_secs as u64);
    match TcpStream::connect_timeout(&format!("{}:{}", host, port).parse().unwrap_or_else(|_| {
        debug!("Invalid address format: {}:{}", host, port);
        "127.0.0.1:0".parse().unwrap() // Fallback that will fail
    }), timeout) {
        Ok(_) => {
            debug!("TCP connection successful to {}:{}", host, port);
            true
        },
        Err(e) => {
            debug!("TCP connection failed to {}:{}: {}", host, port, e);
            false
        }
    }
}

/// Gets the signal strength of a wireless interface
pub fn get_wifi_signal_strength(iface: &str) -> Option<i32> {
    if !iface.starts_with("wl") {
        // Not a wireless interface
        return None;
    }

    let output = Command::new("iwconfig")
        .arg(iface)
        .output()
        .ok()?;
    
    if !output.status.success() {
        return None;
    }

    let stdout = String::from_utf8_lossy(&output.stdout);

    // Extract signal level (dBm)
    for line in stdout.lines() {
        if line.contains("Signal level") {
            if let Some(start) = line.find("Signal level=") {
                let signal_part = &line[start + 13..];
                if let Some(end) = signal_part.find(' ') {
                    let signal_str = &signal_part[..end];
                    if let Ok(signal) = signal_str.parse::<i32>() {
                        debug!("WiFi signal strength for {}: {} dBm", iface, signal);
                        return Some(signal);
                    }
                }
            }
        }
    }

    None
}

/// Checks if an interface is wireless
pub fn is_wireless_interface(iface: &str) -> bool {
    let output = Command::new("ls")
        .args(["-la", &format!("/sys/class/net/{}/wireless", iface)])
        .output();
    
    match output {
        Ok(o) => o.status.success(),
        Err(_) => false
    }
}

/// Returns the IP addresses assigned to an interface
pub fn get_interface_addresses(iface: &str) -> Vec<String> {
    let output = Command::new("ip")
        .args(["addr", "show", "dev", iface])
        .output();
    
    match output {
        Ok(o) => {
            if !o.status.success() {
                return Vec::new();
            }
        
            let stdout = String::from_utf8_lossy(&o.stdout);
            let mut addresses = Vec::new();
        
            // Parse IP addresses
            for line in stdout.lines() {
                if line.trim().starts_with("inet ") {
                    if let Some(ip_with_cidr) = line.split_whitespace().nth(1) {
                        addresses.push(ip_with_cidr.to_string());
                    }
                }
            }
        
            debug!("Interface {} has addresses: {:?}", iface, addresses);
            addresses
        },
        Err(_) => Vec::new()
    }
}