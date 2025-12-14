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
    #[clap(short = 't', long)]
    interval: Option<u64>,

    /// Number of ping attempts
    #[clap(short = 'n', long)]
    count: Option<u8>,

    /// Ping timeout in seconds
    #[clap(long)]
    timeout: Option<u8>,

    /// Speed test interval in seconds (default: 3600 = 1 hour)
    #[clap(long)]
    speedtest_interval: Option<u64>,

    /// Speed threshold percentage to switch to faster interface (default: 35)
    #[clap(long)]
    speed_threshold: Option<u8>,

    /// Number of ping attempts for speed tests (default: 3)
    #[clap(long)]
    speed_test_count: Option<u8>,

    /// Ping timeout in seconds for speed tests (default: 5)
    #[clap(long)]
    speed_test_timeout: Option<u8>,
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
    speed_test_count: Option<u8>,
    speed_test_timeout: Option<u8>,
}

/// Speed test results for an interface
#[derive(Debug, Clone)]
struct SpeedTestResult {
    interface: String,
    download_speed: f64, // in Mbps
}

/// Context for failover operations grouping all necessary parameters
#[derive(Debug, Clone)]
struct FailoverContext {
    wg_interface: String,
    primary: String,
    secondary: String,
    peer_ip: String,
    count: u8,
    timeout: u8,
    speed_threshold: u8,
    speed_test_count: u8,
    speed_test_timeout: u8,
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

    info!("Speed test results for {}: {:.2} Mbps down",
           iface, download_speed);

    Some(SpeedTestResult {
        interface: iface.to_string(),
        download_speed,
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
fn perform_failover_check(context: &FailoverContext) -> Result<()> {
    log_with_timestamp("🔄 Performing failover check");
    
    // Check current interface states
    let primary_up = is_interface_up(&context.primary);
    let secondary_up = is_interface_up(&context.secondary);
    
    info!("Interface states - {}: {}, {}: {}", 
          context.primary, if primary_up { "UP" } else { "DOWN" },
          context.secondary, if secondary_up { "UP" } else { "DOWN" });
    
    // Test connectivity for currently up interfaces
    let primary_connectivity = if primary_up { 
        ping_interface(&context.primary, &context.peer_ip, context.count, context.timeout) 
    } else { 
        false 
    };
    
    let secondary_connectivity = if secondary_up { 
        ping_interface(&context.secondary, &context.peer_ip, context.count, context.timeout) 
    } else { 
        false 
    };
    
    info!("Connectivity - {}: {}, {}: {}", 
          context.primary, if primary_connectivity { "OK" } else { "FAIL" },
          context.secondary, if secondary_connectivity { "OK" } else { "FAIL" });
    
    // Handle connectivity failures
    if !primary_connectivity && primary_up {
        log_with_timestamp(&format!("❌ Primary interface {} lost connectivity", context.primary));
        bring_interface_down(&context.primary)?;
    }
    
    if !secondary_connectivity && secondary_up {
        log_with_timestamp(&format!("❌ Secondary interface {} lost connectivity", context.secondary));
        bring_interface_down(&context.secondary)?;
    }
    
    // Handle interface recovery
    if !primary_up && !primary_connectivity {
        // Try to bring up primary if it's down and we don't know its connectivity
        log_with_timestamp(&format!("🔄 Attempting to recover interface {}", context.primary));
        if let Ok(()) = bring_interface_up(&context.primary) {
            thread::sleep(time::Duration::from_secs(3));
            let connectivity = ping_interface(&context.primary, &context.peer_ip, context.count, context.timeout);
            if !connectivity {
                bring_interface_down(&context.primary)?;
            }
        }
    }
    
    if !secondary_up && !secondary_connectivity {
        // Try to bring up secondary if it's down and we don't know its connectivity
        log_with_timestamp(&format!("🔄 Attempting to recover interface {}", context.secondary));
        if let Ok(()) = bring_interface_up(&context.secondary) {
            thread::sleep(time::Duration::from_secs(3));
            let connectivity = ping_interface(&context.secondary, &context.peer_ip, context.count, context.timeout);
            if !connectivity {
                bring_interface_down(&context.secondary)?;
            }
        }
    }
    
    // Determine which interfaces are currently working
    let current_primary_up = is_interface_up(&context.primary);
    let current_secondary_up = is_interface_up(&context.secondary);
    
    let primary_working = current_primary_up && ping_interface(&context.primary, &context.peer_ip, context.count, context.timeout);
    let secondary_working = current_secondary_up && ping_interface(&context.secondary, &context.peer_ip, context.count, context.timeout);
    
    // Speed optimization logic
    if primary_working && secondary_working {
        log_with_timestamp("⚡ Both interfaces working - performing speed optimization");
        
        let primary_speed = perform_speed_test(&context.primary, &context.peer_ip, context.speed_test_count, context.speed_test_timeout);
        let secondary_speed = perform_speed_test(&context.secondary, &context.peer_ip, context.speed_test_count, context.speed_test_timeout);
        
        if let (Some(primary_result), Some(secondary_result)) = (primary_speed, secondary_speed) {
            if let Some(faster_interface) = should_switch_to_faster_interface(
                &primary_result, 
                &secondary_result, 
                context.speed_threshold
            ) {
                if faster_interface == context.secondary {
                    log_with_timestamp(&format!("🚀 Switching to faster interface: {}", context.secondary));
                    bring_interface_down(&context.primary)?;
                } else {
                    log_with_timestamp(&format!("🚀 Keeping primary interface: {}", context.primary));
                    bring_interface_down(&context.secondary)?;
                }
                
                // Restart WireGuard to pick up the new interface
                restart_wireguard_interface(&context.wg_interface)?;
            }
        }
    } else if !primary_working && secondary_working {
        // Only secondary is working
        log_with_timestamp(&format!("🔄 Switching to secondary interface: {}", context.secondary));
        if current_primary_up {
            bring_interface_down(&context.primary)?;
        }
        bring_interface_up(&context.secondary)?;
        restart_wireguard_interface(&context.wg_interface)?;
    } else if primary_working && !secondary_working {
        // Only primary is working
        log_with_timestamp(&format!("✅ Primary interface working: {}", context.primary));
        if current_secondary_up {
            bring_interface_down(&context.secondary)?;
        }
        // Ensure primary is up
        if !current_primary_up {
            bring_interface_up(&context.primary)?;
            restart_wireguard_interface(&context.wg_interface)?;
        }
    } else {
        // No interfaces working
        log_with_timestamp("❌ No working interfaces found");
        // Try to recover primary interface
        log_with_timestamp(&format!("🔄 Attempting emergency recovery of {}", context.primary));
        if let Ok(()) = bring_interface_up(&context.primary) {
            thread::sleep(time::Duration::from_secs(5));
            restart_wireguard_interface(&context.wg_interface)?;
        }
    }
    
    Ok(())
}

/// Initialize interfaces at startup
fn initialize_interfaces(context: &FailoverContext) -> Result<()> {
    log_with_timestamp("🚀 Initializing interfaces for failover");
    
    // Ensure WireGuard is up
    if !is_interface_up(&context.wg_interface) {
        log_with_timestamp(&format!("🔺 Bringing up WireGuard interface {}", context.wg_interface));
        Command::new("nmcli")
            .args(["connection", "up", &context.wg_interface])
            .output()
            .context("Failed to bring up WireGuard interface")?;
        thread::sleep(time::Duration::from_secs(3));
    }
    
    // Test primary interface first
    log_with_timestamp(&format!("🔍 Testing primary interface {}", context.primary));
    let primary_connectivity = ping_interface(&context.primary, &context.peer_ip, context.count, context.timeout);
    
    if primary_connectivity {
        log_with_timestamp(&format!("✅ Primary interface {} is working", context.primary));
        // Ensure primary is up and secondary is down
        if !is_interface_up(&context.primary) {
            bring_interface_up(&context.primary)?;
        }
        if is_interface_up(&context.secondary) {
            bring_interface_down(&context.secondary)?;
        }
    } else {
        log_with_timestamp(&format!("❌ Primary interface {} failed, testing secondary", context.primary));
        // Test secondary interface
        let secondary_connectivity = ping_interface(&context.secondary, &context.peer_ip, context.count, context.timeout);
        
        if secondary_connectivity {
            log_with_timestamp(&format!("✅ Secondary interface {} is working", context.secondary));
            // Bring up secondary and down primary
            bring_interface_up(&context.secondary)?;
            if is_interface_up(&context.primary) {
                bring_interface_down(&context.primary)?;
            }
            restart_wireguard_interface(&context.wg_interface)?;
        } else {
            log_with_timestamp("❌ No working interfaces found at startup");
            // Emergency: try to bring up primary anyway
            bring_interface_up(&context.primary)?;
            restart_wireguard_interface(&context.wg_interface)?;
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
    
    // Use command line args or config values for monitoring parameters
    let interval = args.interval.or(config.monitoring_config.interval).unwrap_or(30);
    let count = args.count.unwrap_or(3);
    let timeout = args.timeout.unwrap_or(2);
    let speedtest_interval = args.speedtest_interval.or(config.monitoring_config.speedtest_interval).unwrap_or(300);
    let speed_threshold = args.speed_threshold.or(config.monitoring_config.speed_threshold).unwrap_or(20);
    let speed_test_count = args.speed_test_count.or(config.monitoring_config.speed_test_count).unwrap_or(5);
    let speed_test_timeout = args.speed_test_timeout.or(config.monitoring_config.speed_test_timeout).unwrap_or(5);
    
    info!("WireGuard Failover starting...");
    info!("Peer IP: {}", peer_ip);
    info!("WireGuard Interface: {}", wg_interface);
    info!("Primary Interface: {}", primary);
    info!("Secondary Interface: {}", secondary);
    info!("Check Interval: {}s", interval);
    info!("Speed Test Interval: {}s", speedtest_interval);
    
    // Create failover context
    let context = FailoverContext {
        wg_interface: wg_interface.clone(),
        primary: primary.clone(),
        secondary: secondary.clone(),
        peer_ip: peer_ip.clone(),
        count,
        timeout,
        speed_threshold,
        speed_test_count,
        speed_test_timeout,
    };
    
    // Initialize interfaces
    initialize_interfaces(&context)?;
    
    let mut last_speed_test = std::time::Instant::now();

    
    // Main monitoring loop
    loop {
        let now = std::time::Instant::now();
        
        // Check if we should perform speed test
        let should_speed_test = now.duration_since(last_speed_test).as_secs() >= speedtest_interval;
        
        // Create context for this check (adjust speed test parameters if needed)
        let mut check_context = context.clone();
        if !should_speed_test {
            check_context.speed_test_count = 1;
            check_context.speed_test_timeout = timeout;
        }
        
        // Perform failover check
        if let Err(e) = perform_failover_check(&check_context) {
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