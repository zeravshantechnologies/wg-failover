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