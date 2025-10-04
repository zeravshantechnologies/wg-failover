use anyhow::{Context, Result};
use chrono::Local;
use clap::Parser;
use log::{error, info, debug, warn};
use serde::Deserialize;
use std::{thread, time};
use std::process::exit;
use std::process::Command;
use std::env;
use std::path::PathBuf;

/// WireGuard Failover - A utility for ensuring optimal WireGuard VPN connectivity
/// by managing routes based on interface speed optimization
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

/// Get the current interface being used for the default route
fn get_current_default_interface() -> Option<String> {
    debug!("Checking current default route interface");
    
    let output = Command::new("ip")
        .args(["route", "show", "default"])
        .output()
        .ok()?;

    let routes = String::from_utf8_lossy(&output.stdout);
    debug!("Raw default route output: {}", routes);
    
    // Look for the interface in the default route with the lowest metric
    let mut best_route: Option<(String, u32)> = None;
    
    for line in routes.lines() {
        if line.starts_with("default") && line.contains("dev") {
            let parts: Vec<&str> = line.split_whitespace().collect();
            let mut interface = None;
            let mut metric = u32::MAX;
            
            for (i, part) in parts.iter().enumerate() {
                if part == &"dev" && i + 1 < parts.len() {
                    interface = Some(parts[i + 1].to_string());
                } else if part == &"metric" && i + 1 < parts.len() {
                    if let Ok(m) = parts[i + 1].parse() {
                        metric = m;
                    }
                }
            }
            
            if let Some(iface) = interface {
                // Skip WireGuard interfaces and loopback
                if !iface.starts_with("wg") && !iface.starts_with("lo") {
                    if metric < best_route.as_ref().map(|(_, m)| *m).unwrap_or(u32::MAX) {
                        best_route = Some((iface, metric));
                    }
                }
            }
        }
    }
    
    if let Some((iface, metric)) = best_route {
        debug!("Found default route interface: {} (metric: {})", iface, metric);
        Some(iface)
    } else {
        debug!("No default route interface found");
        None
    }
}

/// Switch the default route to use the specified interface
fn switch_default_route(iface: &str) -> Result<()> {
    let gateway = get_gateway_for_interface(iface)
        .context(format!("Failed to find gateway for {}", iface))?;
    
    debug!("Switching default route to interface {} via {}", iface, gateway);

    // Delete all existing default routes
    let _ = Command::new("ip")
        .args(["route", "del", "default"])
        .output();
    
    // Add new default route
    Command::new("ip")
        .args(["route", "add", "default", "via", &gateway, "dev", iface])
        .output()
        .context("Failed to add default route")?;
        
    debug!("Successfully switched default route to interface {}", iface);
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

/// Perform speed test by measuring ping latency and estimating throughput
fn perform_speed_test(iface: &str, peer_ip: &str) -> Option<SpeedTestResult> {
    info!("Performing speed test on interface: {} to peer: {}", iface, peer_ip);
    
    // Emergency debugging
    debug!("DEBUG: Starting speed test for interface {}", iface);
    
    // Use a simpler approach - just measure basic ping performance
    let ping_command = Command::new("ping")
        .args(["-I", iface, "-c", "3", "-W", "5", peer_ip])
        .output();
    
    debug!("DEBUG: Ping command executed for interface {}", iface);

    let output = match ping_command {
        Ok(output) => {
            debug!("DEBUG: Ping command succeeded for {}, status: {}", iface, output.status);
            output
        },
        Err(e) => {
            warn!("Failed to execute ping command for {}: {}", iface, e);
            return None;
        }
    };

    if !output.status.success() {
        warn!("Ping command failed for {} with status: {}", iface, output.status);
        let stderr = String::from_utf8_lossy(&output.stderr);
        debug!("DEBUG: Ping stderr for {}: {}", iface, stderr);
        return None;
    }

    let output_str = String::from_utf8_lossy(&output.stdout);
    debug!("DEBUG: Ping stdout for {}: {}", iface, output_str);
    
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
    let success_rate = success_count as f64 / 3.0;
    
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
    current_interface: &str,
    speed_threshold: u8,
) -> Option<String> {
    let primary_speed = primary_result.download_speed;
    let secondary_speed = secondary_result.download_speed;

    info!("Speed comparison - Primary: {:.2} Mbps, Secondary: {:.2} Mbps", 
          primary_speed, secondary_speed);

    // If current interface is primary and secondary is significantly faster
    if current_interface == primary_result.interface && secondary_speed > 0.0 && primary_speed > 0.0 {
        let speed_improvement = ((secondary_speed - primary_speed) / primary_speed) * 100.0;
        if speed_improvement >= speed_threshold as f64 {
            info!("Secondary interface is {:.1}% faster than primary (threshold: {}%)", 
                  speed_improvement, speed_threshold);
            return Some(secondary_result.interface.clone());
        }
    }
    // If current interface is secondary and primary is faster
    else if current_interface == secondary_result.interface && primary_speed > secondary_speed && secondary_speed > 0.0 {
        info!("Primary interface is faster than secondary");
        return Some(primary_result.interface.clone());
    }

    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_should_switch_to_faster_interface() {
        let primary_result = SpeedTestResult {
            interface: "eth0".to_string(),
            download_speed: 50.0,
            upload_speed: 10.0,
            latency: 20.0,
        };

        let secondary_result = SpeedTestResult {
            interface: "wlan0".to_string(),
            download_speed: 80.0,
            upload_speed: 15.0,
            latency: 30.0,
        };

        // Test: Current on primary, secondary is 60% faster (above 35% threshold)
        let result = should_switch_to_faster_interface(
            &primary_result,
            &secondary_result,
            "eth0",
            35,
        );
        assert_eq!(result, Some("wlan0".to_string()));

        // Test: Current on primary, secondary is only 20% faster (below 35% threshold)
        let secondary_result_slow = SpeedTestResult {
            interface: "wlan0".to_string(),
            download_speed: 60.0,
            upload_speed: 12.0,
            latency: 25.0,
        };
        let result = should_switch_to_faster_interface(
            &primary_result,
            &secondary_result_slow,
            "eth0",
            35,
        );
        assert_eq!(result, None);

        // Test: Current on secondary, primary is slower (should not switch)
        let result = should_switch_to_faster_interface(
            &primary_result,
            &secondary_result,
            "wlan0",
            35,
        );
        assert_eq!(result, None);

        // Test: Current on secondary, primary is faster
        let primary_result_fast = SpeedTestResult {
            interface: "eth0".to_string(),
            download_speed: 100.0,
            upload_speed: 20.0,
            latency: 15.0,
        };
        let secondary_result_slow = SpeedTestResult {
            interface: "wlan0".to_string(),
            download_speed: 80.0,
            upload_speed: 15.0,
            latency: 30.0,
        };
        let result = should_switch_to_faster_interface(
            &primary_result_fast,
            &secondary_result_slow,
            "wlan0",
            35,
        );
        assert_eq!(result, Some("eth0".to_string()));

        // Test: Current on primary, secondary has 0 speed (should not switch)
        let secondary_result_zero = SpeedTestResult {
            interface: "wlan0".to_string(),
            download_speed: 0.0,
            upload_speed: 0.0,
            latency: 0.0,
        };
        let result = should_switch_to_faster_interface(
            &primary_result,
            &secondary_result_zero,
            "eth0",
            35,
        );
        assert_eq!(result, None);
    }
}

fn main() -> Result<()> {
    // Initialize logging
    env_logger::init();
    
    // Parse command line arguments
    let args = Args::parse();
    
    // Load configuration from CLI, environment, or default location
    let config = if let Some(config_path) = args.config {
        info!("Loading configuration from: {}", config_path.display());
        let config_str = std::fs::read_to_string(&config_path)
            .context(format!("Failed to read config file: {}", config_path.display()))?;
        toml::from_str::<Config>(&config_str)
            .context("Failed to parse config file")?
    } else if let Ok(path) = env::var("WG_FAILOVER_CONFIG") {
        info!("Loading configuration from environment: {}", path);
        let config_str = std::fs::read_to_string(&path)
            .context(format!("Failed to read config file: {}", path))?;
        toml::from_str::<Config>(&config_str)
            .context("Failed to parse config file")?
    } else {
        info!("Using CLI arguments");
        Config {
            peer_config: PeerConfig {
                ip: args.peer_ip.context("--peer-ip required when no config file is specified")?,
                count: Some(args.count),
                timeout: Some(args.timeout),
            },
            wireguard_config: WireguardConfig {
                interface: args.wg_interface.context("--wg-interface required when no config file is specified")?,
            },
            interface_config: InterfaceConfig {
                primary: args.primary.context("--primary required when no config file is specified")?,
                secondary: args.secondary.context("--secondary required when no config file is specified")?,
            },
            monitoring_config: MonitoringConfig {
                interval: Some(args.interval),
                speedtest_interval: Some(args.speedtest_interval),
                speed_threshold: Some(args.speed_threshold),
            },
        }
    };
    
    // Extract parameters from config
    let peer_ip = config.peer_config.ip;
    let wg_interface = config.wireguard_config.interface;
    let primary = config.interface_config.primary;
    let secondary = config.interface_config.secondary;
    let interval = config.monitoring_config.interval.unwrap_or(args.interval);
    let speedtest_interval = config.monitoring_config.speedtest_interval.unwrap_or(args.speedtest_interval);
    let speed_threshold = config.monitoring_config.speed_threshold.unwrap_or(args.speed_threshold);
    let count = config.peer_config.count.unwrap_or(args.count);
    let timeout = config.peer_config.timeout.unwrap_or(args.timeout);
    
    info!("WireGuard Failover started");
    info!("Configuration:");
    info!("  Peer IP: {}", peer_ip);
    info!("  WireGuard Interface: {}", wg_interface);
    info!("  Primary Interface: {}", primary);
    info!("  Secondary Interface: {}", secondary);
    info!("  Check Interval: {} seconds", interval);
    info!("  Speed Test Interval: {} seconds", speedtest_interval);
    info!("  Speed Threshold: {}%", speed_threshold);
    
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
    
    let mut last_speed_test = std::time::Instant::now();
    
    // Debug: Check if we're entering the main loop
    debug!("Configuration loaded successfully, entering main loop");
    
    // Main monitoring loop
    loop {
        debug!("Main loop iteration started");
        let current_time = std::time::Instant::now();
        
        // Perform speed tests only when interval has elapsed
        let elapsed = current_time.duration_since(last_speed_test).as_secs();
        let should_run_speed_test = elapsed >= speedtest_interval;
        debug!("Speed test check - elapsed: {}s, interval: {}s, should_run: {}", 
               elapsed, speedtest_interval, should_run_speed_test);
        
        if should_run_speed_test {
            info!("Performing periodic speed tests...");
            debug!("DEBUG: Speed test condition met - calling perform_speed_test functions");
            
            debug!("DEBUG: Calling perform_speed_test for primary interface");
            let primary_result = perform_speed_test(&primary, &peer_ip);
            debug!("DEBUG: Primary speed test result: {:?}", primary_result.is_some());
            
            debug!("DEBUG: Calling perform_speed_test for secondary interface");
            let secondary_result = perform_speed_test(&secondary, &peer_ip);
            debug!("DEBUG: Secondary speed test result: {:?}", secondary_result.is_some());
            
            // Handle speed test results even if one interface fails
            debug!("Processing speed test results - primary: {:?}, secondary: {:?}", 
                   primary_result.is_some(), secondary_result.is_some());
            match (primary_result, secondary_result) {
                (Some(primary_res), Some(secondary_res)) => {
                    last_speed_test = current_time;
                    
                    log_with_timestamp("📊 Speed test results:");
                    info!("🎯 PRIMARY INTERFACE ({})", primary);
                    info!("   Download: {:.2} Mbps", primary_res.download_speed);
                    info!("   Upload: {:.2} Mbps", primary_res.upload_speed);
                    info!("   Latency: {:.1} ms", primary_res.latency);
                    info!("");
                    info!("🔄 SECONDARY INTERFACE ({})", secondary);
                    info!("   Download: {:.2} Mbps", secondary_res.download_speed);
                    info!("   Upload: {:.2} Mbps", secondary_res.upload_speed);
                    info!("   Latency: {:.1} ms", secondary_res.latency);
                    info!("");
                    
                    // Check if we should switch based on speed
                    info!("📈 Speed comparison summary:");
                    info!("   Current interface: {:?}", get_current_default_interface());
                    info!("   Primary download: {:.2} Mbps", primary_res.download_speed);
                    info!("   Secondary download: {:.2} Mbps", secondary_res.download_speed);
                    info!("   Speed threshold: {}%", speed_threshold);
                    if let Some(current_iface) = get_current_default_interface() {
                        if let Some(target_iface) = should_switch_to_faster_interface(
                            &primary_res, 
                            &secondary_res, 
                            &current_iface,
                            speed_threshold,
                        ) {
                            log_with_timestamp(&format!("🚀 Switching to faster interface: {}", target_iface));
                            if let Err(e) = switch_default_route(&target_iface) {
                                error!("Failed to switch to faster interface: {}", e);
                            } else {
                                info!("Successfully switched to faster interface: {}", target_iface);
                            }
                        } else {
                            info!("No significant speed improvement detected, keeping current interface");
                        }
                    } else {
                        warn!("Could not determine current interface for speed-based switching");
                    }
                }
                (Some(primary_res), None) => {
                    last_speed_test = current_time;
                    log_with_timestamp("📊 Speed test results (secondary interface failed):");
                    info!("🎯 PRIMARY INTERFACE ({})", primary);
                    info!("   Download: {:.2} Mbps", primary_res.download_speed);
                    info!("   Upload: {:.2} Mbps", primary_res.upload_speed);
                    info!("   Latency: {:.1} ms", primary_res.latency);
                    info!("");
                    info!("❌ SECONDARY INTERFACE ({}) - FAILED", secondary);
                    warn!("Secondary interface speed test failed - using primary interface");
                }
                (None, Some(secondary_res)) => {
                    last_speed_test = current_time;
                    log_with_timestamp("📊 Speed test results (primary interface failed):");
                    info!("❌ PRIMARY INTERFACE ({}) - FAILED", primary);
                    info!("");
                    info!("🔄 SECONDARY INTERFACE ({})", secondary);
                    info!("   Download: {:.2} Mbps", secondary_res.download_speed);
                    info!("   Upload: {:.2} Mbps", secondary_res.upload_speed);
                    info!("   Latency: {:.1} ms", secondary_res.latency);
                    warn!("Primary interface speed test failed - using secondary interface");
                }
                (None, None) => {
                    warn!("Speed tests failed for both interfaces");
                }
            }
        }
        
        // Check interface connectivity (for monitoring only - no automatic switching)
        info!("Checking primary interface: {}", primary);
        let primary_ok = ping_interface(&primary, &peer_ip, count, timeout);
        info!("Primary interface {} connectivity to {}: {}",
            primary, peer_ip, if primary_ok { "OK" } else { "FAIL" });
        
        info!("Checking secondary interface: {}", secondary);
        let secondary_ok = ping_interface(&secondary, &peer_ip, count, timeout);
        info!("Secondary interface {} connectivity to {}: {}",
            secondary, peer_ip, if secondary_ok { "OK" } else { "FAIL" });
        
        // Get current default route interface
        let current_iface = get_current_default_interface();
        info!("Current default route interface: {:?}",
            current_iface.as_deref().unwrap_or("unknown"));
        
        // Only handle connectivity issues for logging - let WireGuard handle failover
        match (primary_ok, secondary_ok) {
            (true, true) => {
                debug!("Both interfaces are working correctly");
            }
            (false, true) => {
                warn!("Primary interface is down, but secondary is working");
            }
            (true, false) => {
                warn!("Secondary interface is down, but primary is working");
            }
            (false, false) => {
                error!("Both interfaces are unreachable");
            }
        }

        thread::sleep(time::Duration::from_secs(interval));
    }
}