use anyhow::{Context, Result};
use chrono::Local;
use clap::Parser;
use log::{error, info, debug, warn};
use serde::Deserialize;
use std::{thread, time};

use std::process::Command;

use std::path::PathBuf;

/// WireGuard Failover - A utility for ensuring optimal WireGuard VPN connectivity
/// by managing interface state based on connectivity and speed optimization
#[derive(Parser, Debug)]
#[clap(author, version, about)]
struct Args {
    /// The IP address or hostname of the WireGuard peer
    #[clap(short = 'i', long)]
    peer_ip: Option<String>,

    /// Path to configuration file
    #[clap(long)]
    config: Option<PathBuf>,

    /// WireGuard interface name (e.g., wg0)
    #[clap(short = 'w', long)]
    wg_interface: Option<String>,

    /// Primary network interface (e.g., eth0, enp0s31f6)
    #[clap(short = 'p', long)]
    primary: Option<String>,

    /// Secondary network interface (e.g., wlan0, wlp0s20f0u5)
    #[clap(short = 's', long)]
    secondary: Option<String>,

    /// Connectivity check interval in seconds
    #[clap(short = 't', long, default_value = "30")]
    interval: u64,

    /// Number of ping attempts
    #[clap(short = 'n', long, default_value = "2")]
    count: u8,

    /// Ping timeout in seconds
    #[clap(long, default_value = "2")]
    timeout: u8,

    /// Speed test interval in seconds (default: 3600 = 1 hour)
    #[clap(long, default_value = "3600")]
    speedtest_interval: u64,

    /// Speed threshold percentage to switch to faster interface (default: 35)
    #[clap(long, default_value = "35")]
    speed_threshold: u8,

    /// Minimum switch interval in seconds to prevent flapping (default: 30)
    #[clap(long, default_value = "30")]
    min_switch_interval: u64,

    /// Number of ping attempts for speed tests (default: 3)
    #[clap(long, default_value = "3")]
    speed_test_count: u8,

    /// Ping timeout in seconds for speed tests (default: 5)
    #[clap(long, default_value = "5")]
    speed_test_timeout: u8,
}

/// Configuration file structure
#[derive(Debug, Deserialize)]
struct Config {
    #[serde(rename = "peer")]
    peer_config: PeerConfig,
    #[serde(rename = "wireguard")]
    wireguard_config: WireguardConfig,
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
struct WireguardConfig {
    interface: String,
}

#[derive(Debug, Deserialize)]
struct InterfaceConfig {
    primary: String,
    secondary: String,
}

#[derive(Debug, Deserialize)]
struct MonitoringConfig {
    interval: Option<u64>,
    speedtest_interval: Option<u64>,
    speed_threshold: Option<u8>,
    min_switch_interval: Option<u64>,
    speed_test_count: Option<u8>,
    speed_test_timeout: Option<u8>,
}

/// Speed test results for an interface
#[derive(Debug, Clone)]
struct SpeedTestResult {
    interface: String,
    download_speed: f64, // in Mbps
    upload_speed: f64,   // in Mbps
    latency: f64,        // in ms
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

/// Bring interface up using nmcli
fn bring_interface_up(iface: &str) -> Result<()> {
    log_with_timestamp(&format!("🔺 Bringing interface {} up", iface));
    
    Command::new("nmcli")
        .args(["device", "connect", iface])
        .output()
        .context(format!("Failed to bring interface {} up", iface))?;
    
    // Wait a moment for interface to stabilize
    thread::sleep(time::Duration::from_secs(2));
    Ok(())
}

/// Bring interface down using nmcli
fn bring_interface_down(iface: &str) -> Result<()> {
    log_with_timestamp(&format!("🔻 Bringing interface {} down", iface));
    
    Command::new("nmcli")
        .args(["device", "disconnect", iface])
        .output()
        .context(format!("Failed to bring interface {} down", iface))?;
    
    // Wait a moment for interface to fully disconnect
    thread::sleep(time::Duration::from_secs(2));
    Ok(())
}

/// Restart WireGuard interface using nmcli
fn restart_wireguard_interface(wg_interface: &str) -> Result<()> {
    log_with_timestamp(&format!("🔄 Restarting WireGuard interface {}", wg_interface));
    
    // First bring down
    Command::new("nmcli")
        .args(["connection", "down", wg_interface])
        .output()
        .context(format!("Failed to bring down WireGuard interface {}", wg_interface))?;
    
    // Wait for cleanup
    thread::sleep(time::Duration::from_secs(1));
    
    // Then bring up
    Command::new("nmcli")
        .args(["connection", "up", wg_interface])
        .output()
        .context(format!("Failed to bring up WireGuard interface {}", wg_interface))?;
    
    // Wait for WireGuard to establish connection
    thread::sleep(time::Duration::from_secs(3));
    Ok(())
}

/// Check if interface is up using nmcli
fn is_interface_up(iface: &str) -> bool {
    let output = Command::new("nmcli")
        .args(["device", "status"])
        .output();
    
    match output {
        Ok(o) => {
            let status = String::from_utf8_lossy(&o.stdout);
            for line in status.lines() {
                if line.contains(iface) && line.contains("connected") {
                    return true;
                }
            }
            false
        },
        Err(_) => false
    }
}

/// Perform speed test by measuring ping latency and estimating throughput
fn perform_speed_test(iface: &str, peer_ip: &str, speed_test_count: u8, speed_test_timeout: u8) -> Option<SpeedTestResult> {
    info!("Performing speed test on interface: {} to peer: {}", iface, peer_ip);

    let ping_command = Command::new("ping")
        .args(["-I", iface, "-c", &speed_test_count.to_string(), "-W", &speed_test_timeout.to_string(), peer_ip])
        .output();

    let output = match ping_command {
        Ok(output) => output,
        Err(e) => {
            warn!("Failed to execute ping command for {}: {}", iface, e);
            return None;
        }
    };

    if !output.status.success() {
        warn!("Ping command failed for {} with status: {}", iface, output.status);
        return None;
    }

    let output_str = String::from_utf8_lossy(&output.stdout);

    // Extract latency from ping output
    let mut latency = 0.0;
    for line in output_str.lines() {
        if line.contains("avg") {
            let parts: Vec<&str> = line.split('/').collect();
            if parts.len() >= 5 {
                latency = parts[4].parse::<f64>().unwrap_or(0.0);
                break;
            }
        }
    }

    // Calculate packet success rate
    let success_count = output_str.matches("time=").count();
    let success_rate = success_count as f64 / speed_test_count as f64;

    // Estimate speed based on latency and success rate
    let download_speed = if success_rate > 0.8 {
        if latency < 20.0 { 200.0 }
        else if latency < 50.0 { 100.0 }
        else if latency < 100.0 { 50.0 }
        else { 20.0 }
    } else if success_rate > 0.5 {
        if latency < 100.0 { 30.0 }
        else { 10.0 }
    } else {
        5.0
    };

    let upload_speed = download_speed * 0.8;

    info!("Speed test results for {}: {:.2} Mbps down, {:.2} Mbps up, {:.1} ms latency",
           iface, download_speed, upload_speed, latency);

    Some(SpeedTestResult {
        interface: iface.to_string(),
        download_speed,
        upload_speed,
        latency,
    })
}

/// Compare speed test results and determine if we should switch interfaces
fn should_switch_to_faster_interface(
    primary_result: &SpeedTestResult,
    secondary_result: &SpeedTestResult,
    speed_threshold: u8,
) -> Option<String> {
    let primary_speed = primary_result.download_speed;
    let secondary_speed = secondary_result.download_speed;

    info!("Speed comparison - Primary: {:.2} Mbps, Secondary: {:.2} Mbps",
          primary_speed, secondary_speed);

    // If secondary is significantly faster
    if secondary_speed > 0.0 && primary_speed > 0.0 {
        let speed_improvement = ((secondary_speed - primary_speed) / primary_speed) * 100.0;
        if speed_improvement >= speed_threshold as f64 {
            info!("Secondary interface is {:.1}% faster than primary (threshold: {}%)",
                  speed_improvement, speed_threshold);
            return Some(secondary_result.interface.clone());
        }
    }

    // If primary is faster or speeds are comparable, stick with primary
    None
}

/// Main failover logic using interface up/down approach
fn perform_failover_check(
    wg_interface: &str,
    primary: &str,
    secondary: &str,
    peer_ip: &str,
    count: u8,
    timeout: u8,
    speed_threshold: u8,
    speed_test_count: u8,
    speed_test_timeout: u8,
) -> Result<()> {
    log_with_timestamp("🔄 Performing failover check");
    
    // Check current interface states
    let primary_up = is_interface_up(primary);
    let secondary_up = is_interface_up(secondary);
    
    info!("Interface states - {}: {}, {}: {}", 
          primary, if primary_up { "UP" } else { "DOWN" },
          secondary, if secondary_up { "UP" } else { "DOWN" });
    
    // Test connectivity for currently up interfaces
    let primary_connectivity = if primary_up { 
        ping_interface(primary, peer_ip, count, timeout) 
    } else { 
        false 
    };
    
    let secondary_connectivity = if secondary_up { 
        ping_interface(secondary, peer_ip, count, timeout) 
    } else { 
        false 
    };
    
    info!("Connectivity - {}: {}, {}: {}", 
          primary, if primary_connectivity { "OK" } else { "FAIL" },
          secondary, if secondary_connectivity { "OK" } else { "FAIL" });
    
    // Handle connectivity failures
    if !primary_connectivity && primary_up {
        log_with_timestamp(&format!("❌ Primary interface {} lost connectivity", primary));
        bring_interface_down(primary)?;
    }
    
    if !secondary_connectivity && secondary_up {
        log_with_timestamp(&format!("❌ Secondary interface {} lost connectivity", secondary));
        bring_interface_down(secondary)?;
    }
    
    // Handle interface recovery
    if !primary_up && !primary_connectivity {
        // Try to bring up primary if it's down and we don't know its connectivity
        log_with_timestamp(&format!("🔄 Attempting to recover interface {}", primary));
        if let Ok(()) = bring_interface_up(primary) {
            thread::sleep(time::Duration::from_secs(3));
            let connectivity = ping_interface(primary, peer_ip, count, timeout);
            if !connectivity {
                bring_interface_down(primary)?;
            }
        }
    }
    
    if !secondary_up && !secondary_connectivity {
        // Try to bring up secondary if it's down and we don't know its connectivity
        log_with_timestamp(&format!("🔄 Attempting to recover interface {}", secondary));
        if let Ok(()) = bring_interface_up(secondary) {
            thread::sleep(time::Duration::from_secs(3));
            let connectivity = ping_interface(secondary, peer_ip, count, timeout);
            if !connectivity {
                bring_interface_down(secondary)?;
            }
        }
    }
    
    // Determine which interfaces are currently working
    let current_primary_up = is_interface_up(primary);
    let current_secondary_up = is_interface_up(secondary);
    
    let primary_working = current_primary_up && ping_interface(primary, peer_ip, count, timeout);
    let secondary_working = current_secondary_up && ping_interface(secondary, peer_ip, count, timeout);
    
    // Speed optimization logic
    if primary_working && secondary_working {
        log_with_timestamp("⚡ Both interfaces working - performing speed optimization");
        
        let primary_speed = perform_speed_test(primary, peer_ip, speed_test_count, speed_test_timeout);
        let secondary_speed = perform_speed_test(secondary, peer_ip, speed_test_count, speed_test_timeout);
        
        if let (Some(primary_result), Some(secondary_result)) = (primary_speed, secondary_speed) {
            if let Some(faster_interface) = should_switch_to_faster_interface(
                &primary_result, 
                &secondary_result, 
                speed_threshold
            ) {
                if faster_interface == secondary {
                    log_with_timestamp(&format!("🚀 Switching to faster interface: {}", secondary));
                    bring_interface_down(primary)?;
                } else {
                    log_with_timestamp(&format!("🚀 Keeping primary interface: {}", primary));
                    bring_interface_down(secondary)?;
                }
                
                // Restart WireGuard to pick up the new interface
                restart_wireguard_interface(wg_interface)?;
            }
        }
    } else if !primary_working && secondary_working {
        // Only secondary is working
        log_with_timestamp(&format!("🔄 Switching to secondary interface: {}", secondary));
        if current_primary_up {
            bring_interface_down(primary)?;
        }
        bring_interface_up(secondary)?;
        restart_wireguard_interface(wg_interface)?;
    } else if primary_working && !secondary_working {
        // Only primary is working
        log_with_timestamp(&format!("✅ Primary interface working: {}", primary));
        if current_secondary_up {
            bring_interface_down(secondary)?;
        }
        // Ensure primary is up
        if !current_primary_up {
            bring_interface_up(primary)?;
            restart_wireguard_interface(wg_interface)?;
        }
    } else {
        // No interfaces working
        log_with_timestamp("❌ No working interfaces found");
        // Try to recover primary interface
        log_with_timestamp(&format!("🔄 Attempting emergency recovery of {}", primary));
        if let Ok(()) = bring_interface_up(primary) {
            thread::sleep(time::Duration::from_secs(5));
            restart_wireguard_interface(wg_interface)?;
        }
    }
    
    Ok(())
}

/// Initialize interfaces at startup
fn initialize_interfaces(primary: &str, secondary: &str, wg_interface: &str, peer_ip: &str, count: u8, timeout: u8) -> Result<()> {
    log_with_timestamp("🚀 Initializing interfaces for failover");
    
    // Ensure WireGuard is up
    if !is_interface_up(wg_interface) {
        log_with_timestamp(&format!("🔺 Bringing up WireGuard interface {}", wg_interface));
        Command::new("nmcli")
            .args(["connection", "up", wg_interface])
            .output()
            .context("Failed to bring up WireGuard interface")?;
        thread::sleep(time::Duration::from_secs(3));
    }
    
    // Test primary interface first
    log_with_timestamp(&format!("🔍 Testing primary interface {}", primary));
    let primary_connectivity = ping_interface(primary, peer_ip, count, timeout);
    
    if primary_connectivity {
        log_with_timestamp(&format!("✅ Primary interface {} is working", primary));
        // Ensure primary is up and secondary is down
        if !is_interface_up(primary) {
            bring_interface_up(primary)?;
        }
        if is_interface_up(secondary) {
            bring_interface_down(secondary)?;
        }
    } else {
        log_with_timestamp(&format!("❌ Primary interface {} failed, testing secondary", primary));
        // Test secondary interface
        let secondary_connectivity = ping_interface(secondary, peer_ip, count, timeout);
        
        if secondary_connectivity {
            log_with_timestamp(&format!("✅ Secondary interface {} is working", secondary));
            // Bring up secondary and down primary
            bring_interface_up(secondary)?;
            if is_interface_up(primary) {
                bring_interface_down(primary)?;
            }
            restart_wireguard_interface(wg_interface)?;
        } else {
            log_with_timestamp("❌ No working interfaces found at startup");
            // Emergency: try to bring up primary anyway
            bring_interface_up(primary)?;
            restart_wireguard_interface(wg_interface)?;
        }
    }
    
    Ok(())
}

fn main() -> Result<()> {
    env_logger::init();
    
    let args = Args::parse();
    
    // Load configuration
    let config_path = args.config
        .or_else(|| Some(PathBuf::from("/etc/wg-failover/config.toml")))
        .unwrap();
    
    let config_content = std::fs::read_to_string(&config_path)
        .context(format!("Failed to read config file: {:?}", config_path))?;
    
    let config: Config = toml::from_str(&config_content)
        .context("Failed to parse config file")?;
    
    // Use command line args or config values
    let peer_ip = args.peer_ip.unwrap_or(config.peer_config.ip);
    let wg_interface = args.wg_interface.unwrap_or(config.wireguard_config.interface);
    let primary = args.primary.unwrap_or(config.interface_config.primary);
    let secondary = args.secondary.unwrap_or(config.interface_config.secondary);
    let interval = args.interval;
    let count = args.count;
    let timeout = args.timeout;
    let speedtest_interval = args.speedtest_interval;
    let speed_threshold = args.speed_threshold;

    let speed_test_count = args.speed_test_count;
    let speed_test_timeout = args.speed_test_timeout;
    
    info!("WireGuard Failover starting...");
    info!("Peer IP: {}", peer_ip);
    info!("WireGuard Interface: {}", wg_interface);
    info!("Primary Interface: {}", primary);
    info!("Secondary Interface: {}", secondary);
    info!("Check Interval: {}s", interval);
    info!("Speed Test Interval: {}s", speedtest_interval);
    
    // Initialize interfaces
    initialize_interfaces(&primary, &secondary, &wg_interface, &peer_ip, count, timeout)?;
    
    let mut last_speed_test = std::time::Instant::now();

    
    // Main monitoring loop
    loop {
        let now = std::time::Instant::now();
        
        // Check if we should perform speed test
        let should_speed_test = now.duration_since(last_speed_test).as_secs() >= speedtest_interval;
        
        // Perform failover check
        if let Err(e) = perform_failover_check(
            &wg_interface,
            &primary,
            &secondary,
            &peer_ip,
            count,
            timeout,
            speed_threshold,
            if should_speed_test { speed_test_count } else { 1 },
            if should_speed_test { speed_test_timeout } else { timeout },
        ) {
            error!("Failover check failed: {}", e);
        }
        
        // Update timers
        if should_speed_test {
            last_speed_test = now;
        }
        
        // Sleep until next check
        thread::sleep(time::Duration::from_secs(interval));
    }
}