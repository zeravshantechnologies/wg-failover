use anyhow::{Context, Result};
use chrono::Local;
use clap::Parser;
use log::{error, info, debug, warn};
use serde::Deserialize;
use std::path::PathBuf;
use std::process::Command;
use std::thread;
use std::time::{Duration, Instant};

/// WireGuard Failover - Metric-based routing manager
#[derive(Parser, Debug, Clone)]
#[clap(author, version, about)]
struct Args {
    /// The IP address or hostname of the WireGuard peer
    #[clap(short = 'i', long)]
    peer_ip: Option<String>,

    /// Path to configuration file
    #[clap(long)]
    config: Option<PathBuf>,

    /// Primary network interface (e.g., eth0)
    #[clap(short = 'p', long)]
    primary: Option<String>,

    /// Secondary network interface (e.g., wlan0)
    #[clap(short = 's', long)]
    secondary: Option<String>,

    /// Connectivity check interval in seconds
    #[clap(short = 't', long)]
    interval: Option<u64>,

    /// Speed test interval in seconds
    #[clap(long)]
    speedtest_interval: Option<u64>,

    /// Speed threshold percentage to switch to faster interface
    #[clap(long)]
    speed_threshold: Option<u8>,
}

#[derive(Debug, Deserialize, Clone)]
struct Config {
    peer: Option<PeerConfig>,
    interfaces: Option<InterfaceConfig>,
    monitoring: Option<MonitoringConfig>,
}

#[derive(Debug, Deserialize, Clone)]
struct PeerConfig {
    ip: Option<String>,
}

#[derive(Debug, Deserialize, Clone)]
struct InterfaceConfig {
    primary: Option<String>,
    secondary: Option<String>,
}

#[derive(Debug, Deserialize, Clone)]
struct MonitoringConfig {
    interval: Option<u64>,
    speedtest_interval: Option<u64>,
    speed_threshold: Option<u8>,
}

#[derive(Debug, Clone)]
struct AppState {
    peer_ip: String,
    primary_iface: String,
    secondary_iface: String,
    check_interval: Duration,
    speed_check_interval: Duration,
    speed_threshold: u8,
}

#[derive(Debug, Clone, PartialEq)]
enum InterfaceStatus {
    Working,
    Failed,
    Unknown,
}

#[derive(Debug, Clone)]
struct InterfaceMetrics {
    status: InterfaceStatus,
    connectivity_latency_ms: f64,
    speed_latency_ms: f64,
}

impl Default for InterfaceMetrics {
    fn default() -> Self {
        Self {
            status: InterfaceStatus::Unknown,
            connectivity_latency_ms: 0.0,
            speed_latency_ms: 0.0,
        }
    }
}

fn log_with_timestamp(msg: &str) {
    info!("[{}] {}", Local::now().format("%Y-%m-%d %H:%M:%S"), msg);
}

/// Retrieve the default gateway for a specific interface
fn get_gateway_for_interface(iface: &str) -> Option<String> {
    debug!("get_gateway_for_interface called for interface: {}", iface);
    
    // Run: ip route show dev <iface>
    debug!("Executing command: ip route show dev {}", iface);
    let output = Command::new("ip")
        .args(["route", "show", "dev", iface])
        .output()
        .ok()?;

    let stdout = String::from_utf8_lossy(&output.stdout);
    debug!("Command output (stdout): {}", stdout);
    
    // Look for lines like "default via 192.168.1.1 ..."
    debug!("Parsing output lines for default gateway");
    for line in stdout.lines() {
        debug!("Processing line: {}", line);
        let parts: Vec<&str> = line.split_whitespace().collect();
        if parts.len() >= 3 && parts[0] == "default" && parts[1] == "via" {
            let gateway = parts[2].to_string();
            debug!("Found default gateway: {} for interface {}", gateway, iface);
            return Some(gateway);
        } else {
            debug!("Line does not match default gateway pattern");
        }
    }
    
    // If no default route specifically for this dev, try main table generic check? 
    // Usually "ip route show dev X" is sufficient for connected interfaces with gateways.
    // If it's a P2P link, gateway might not be needed.
    debug!("No default gateway found for interface {}", iface);
    None
}

/// Execute ping and return stats (success, avg_latency_ms)
fn measure_latency(iface: &str, target: &str, count: u8, timeout: u8) -> (bool, f64) {
    debug!("measure_latency called: iface={}, target={}, count={}, timeout={}", iface, target, count, timeout);
    
    let cmd_str = format!("ping -I {} -c {} -W {} {}", iface, count, timeout, target);
    debug!("Executing command: {}", cmd_str);
    
    let output = Command::new("ping")
        .args([
            "-I", iface,
            "-c", &count.to_string(),
            "-W", &timeout.to_string(),
            target,
        ])
        .output();

    match output {
        Ok(out) if out.status.success() => {
            debug!("Ping command succeeded with status: {}", out.status);
            let stdout = String::from_utf8_lossy(&out.stdout);
            let stderr = String::from_utf8_lossy(&out.stderr);
            debug!("Ping stdout: {}", stdout);
            if !stderr.is_empty() {
                debug!("Ping stderr: {}", stderr);
            }
            
            // Parse rtt min/avg/max/mdev = 1.1/2.2/3.3/0.4 ms
            debug!("Parsing ping output for latency statistics");
            for line in stdout.lines() {
                debug!("Processing line: {}", line);
                if line.contains("min/avg/max") {
                    debug!("Found latency statistics line: {}", line);
                    if let Some(stats) = line.split('=').nth(1) {
                        let parts: Vec<&str> = stats.split('/').collect();
                        debug!("Parsed statistics parts: {:?}", parts);
                        if parts.len() >= 2 {
                            if let Ok(avg) = parts[1].trim().parse::<f64>() {
                                debug!("Successfully parsed average latency: {} ms", avg);
                                return (true, avg);
                            } else {
                                debug!("Failed to parse average latency from: {}", parts[1].trim());
                            }
                        } else {
                            debug!("Insufficient statistics parts, expected at least 2");
                        }
                    } else {
                        debug!("No statistics found after '=' in line");
                    }
                }
            }
            debug!("Ping succeeded but could not parse latency statistics");
            (true, 0.0) // Success but failed to parse latency?
        }
        Ok(out) => {
            debug!("Ping command failed with status: {}", out.status);
            let stderr = String::from_utf8_lossy(&out.stderr);
            debug!("Ping stderr: {}", stderr);
            (false, 0.0)
        }
        Err(e) => {
            debug!("Failed to execute ping command: {}", e);
            (false, 0.0)
        }
    }
}

/// Update the route to the peer IP through the specified interface
fn update_route(peer_ip: &str, iface: &str, gateway: Option<&String>) -> Result<()> {
    debug!("update_route called: peer_ip={}, iface={}, gateway={:?}", peer_ip, iface, gateway);
    
    // Command: ip route replace <peer_ip> [via <gateway>] dev <iface>
    let mut cmd = Command::new("ip");
    cmd.arg("route").arg("replace").arg(peer_ip);
    
    if let Some(gw) = gateway {
        debug!("Adding gateway to route: via {}", gw);
        cmd.arg("via").arg(gw);
    } else {
        debug!("No gateway specified for route");
    }
    
    cmd.arg("dev").arg(iface);
    
    // We can set a metric if we want, but since we are "replacing", 
    // we effectively choose this interface as the active one for this destination.
    // To be cleaner, we can set metric 100.
    cmd.arg("metric").arg("100");
    
    let cmd_str = format!("{:?}", cmd);
    debug!("Executing route command: {}", cmd_str);

    let output = cmd.output().context("Failed to execute ip route command")?;
    
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let stdout = String::from_utf8_lossy(&output.stdout);
        debug!("Route command failed with status: {}", output.status);
        debug!("Route command stderr: {}", stderr);
        debug!("Route command stdout: {}", stdout);
        return Err(anyhow::anyhow!("ip route failed: {}", stderr));
    }
    
    debug!("Route command succeeded with status: {}", output.status);
    let stdout = String::from_utf8_lossy(&output.stdout);
    if !stdout.is_empty() {
        debug!("Route command stdout: {}", stdout);
    }
    let stderr = String::from_utf8_lossy(&output.stderr);
    if !stderr.is_empty() {
        debug!("Route command stderr: {}", stderr);
    }
    
    debug!("Updated route for {} via {} (gw: {:?})", peer_ip, iface, gateway);
    Ok(())
}

fn main() -> Result<()> {
    env_logger::init();
    // Note: For detailed debug logging, set environment variable RUST_LOG=debug
    log_with_timestamp("Logger initialized");
    
    // 1. Load Configuration
    log_with_timestamp("Parsing command line arguments");
    let args = Args::parse();
    log_with_timestamp(&format!("Command line arguments parsed: {:?}", args));
    
    log_with_timestamp("Determining configuration file path");
    let config_path = args.config.clone()
        .unwrap_or_else(|| PathBuf::from("/etc/wg-failover/config.toml"));
    log_with_timestamp(&format!("Configuration file path: {:?}", config_path));
        
    let config_file: Option<Config> = if config_path.exists() {
        log_with_timestamp(&format!("Configuration file exists, reading from {:?}", config_path));
        let content = std::fs::read_to_string(&config_path)
            .context(format!("Failed to read config file {:?}", config_path))?;
        log_with_timestamp("Configuration file read successfully, parsing TOML");
        Some(toml::from_str(&content).context("Failed to parse TOML")?)
    } else {
        log_with_timestamp("Configuration file does not exist, using command line arguments only");
        None
    };

    // Helper to extract config values with precedence: Args -> Config File -> Defaults
    log_with_timestamp("Extracting configuration values");
    let peer_ip = args.peer_ip.clone()
        .or_else(|| config_file.as_ref().and_then(|c| c.peer.as_ref()).and_then(|p| p.ip.clone()))
        .context("Peer IP is required (in args or config)")?;
    log_with_timestamp(&format!("Peer IP determined: {}", peer_ip));

    let primary_iface = args.primary.clone()
        .or_else(|| config_file.as_ref().and_then(|c| c.interfaces.as_ref()).and_then(|i| i.primary.clone()))
        .context("Primary interface is required")?;
    log_with_timestamp(&format!("Primary interface determined: {}", primary_iface));

    let secondary_iface = args.secondary.clone()
        .or_else(|| config_file.as_ref().and_then(|c| c.interfaces.as_ref()).and_then(|i| i.secondary.clone()))
        .context("Secondary interface is required")?;
    log_with_timestamp(&format!("Secondary interface determined: {}", secondary_iface));
        
    let interval_secs = args.interval
        .or_else(|| config_file.as_ref().and_then(|c| c.monitoring.as_ref()).and_then(|m| m.interval))
        .unwrap_or(30);
    log_with_timestamp(&format!("Check interval determined: {} seconds", interval_secs));

    let speed_interval_secs = args.speedtest_interval
        .or_else(|| config_file.as_ref().and_then(|c| c.monitoring.as_ref()).and_then(|m| m.speedtest_interval))
        .unwrap_or(300);
    log_with_timestamp(&format!("Speed check interval determined: {} seconds", speed_interval_secs));
        
    let speed_threshold = args.speed_threshold
        .or_else(|| config_file.as_ref().and_then(|c| c.monitoring.as_ref()).and_then(|m| m.speed_threshold))
        .unwrap_or(20);
    log_with_timestamp(&format!("Speed threshold determined: {}%", speed_threshold));

    log_with_timestamp("Creating application state");
    let state = AppState {
        peer_ip,
        primary_iface,
        secondary_iface,
        check_interval: Duration::from_secs(interval_secs),
        speed_check_interval: Duration::from_secs(speed_interval_secs),
        speed_threshold,
    };
    log_with_timestamp("Application state created successfully");

    log_with_timestamp(&format!("Starting WireGuard Failover (Simple Metric Mode)"));
    info!("Peer: {}", state.peer_ip);
    info!("Primary: {}, Secondary: {}", state.primary_iface, state.secondary_iface);
    info!("Intervals - Check: {:?}, Speed: {:?}", state.check_interval, state.speed_check_interval);
    log_with_timestamp("Initialization complete, entering main loop");

    log_with_timestamp("Initializing metrics and state variables");
    let mut primary_metrics = InterfaceMetrics::default();
    let mut secondary_metrics = InterfaceMetrics::default();
    log_with_timestamp("Metrics initialized to default values");
    
    // Force check on start if possible, otherwise start timer now
    log_with_timestamp("Setting up speed check timer");
    let mut last_speed_check = Instant::now()
        .checked_sub(state.speed_check_interval)
        .unwrap_or(Instant::now());
    log_with_timestamp(&format!("Last speed check time set to: {:?}", last_speed_check));
    
    let mut current_active_interface: Option<String> = None;
    log_with_timestamp("Current active interface initialized to None");

    loop {
        log_with_timestamp("Starting main loop iteration");
        let now = Instant::now();
        log_with_timestamp(&format!("Current time instant: {:?}", now));
        
        // ----------------------------------------
        // 1. Identify Gateways (Dynamic, in case of network changes)
        // ----------------------------------------
        log_with_timestamp("Identifying gateways for interfaces");
        let primary_gw = get_gateway_for_interface(&state.primary_iface);
        let secondary_gw = get_gateway_for_interface(&state.secondary_iface);
        log_with_timestamp(&format!("Primary gateway: {:?}, Secondary gateway: {:?}", primary_gw, secondary_gw));

        // ----------------------------------------
        // 2. Connectivity Check (Frequent)
        // ----------------------------------------
        // We ping the Peer IP through specific interfaces to test reachability.
        // NOTE: If the interface doesn't have a route to peer, ping -I usually works if gateway is on-link or default exists.
        
        log_with_timestamp("Starting connectivity checks");
        log_with_timestamp(&format!("Checking connectivity via primary interface: {}", state.primary_iface));
        let (p_ok, p_lat) = measure_latency(&state.primary_iface, &state.peer_ip, 1, 2);
        log_with_timestamp(&format!("Primary interface connectivity result: success={}, latency={:.1}ms", p_ok, p_lat));
        
        log_with_timestamp(&format!("Checking connectivity via secondary interface: {}", state.secondary_iface));
        let (s_ok, s_lat) = measure_latency(&state.secondary_iface, &state.peer_ip, 1, 2);
        log_with_timestamp(&format!("Secondary interface connectivity result: success={}, latency={:.1}ms", s_ok, s_lat));

        log_with_timestamp("Updating metrics based on connectivity results");
        primary_metrics.status = if p_ok { InterfaceStatus::Working } else { InterfaceStatus::Failed };
        primary_metrics.connectivity_latency_ms = p_lat;
        log_with_timestamp(&format!("Primary metrics updated: status={:?}, latency={:.1}ms", primary_metrics.status, primary_metrics.connectivity_latency_ms));
        
        secondary_metrics.status = if s_ok { InterfaceStatus::Working } else { InterfaceStatus::Failed };
        secondary_metrics.connectivity_latency_ms = s_lat;
        log_with_timestamp(&format!("Secondary metrics updated: status={:?}, latency={:.1}ms", secondary_metrics.status, secondary_metrics.connectivity_latency_ms));

        // ----------------------------------------
        // 3. Speed Check (Periodic)
        // ----------------------------------------
        // Only if both are working, we might want to run a heavier test to decide best path.
        log_with_timestamp("Checking if speed test is due");
        let time_since_last_speed_check = now.duration_since(last_speed_check);
        log_with_timestamp(&format!("Time since last speed check: {:?}, required interval: {:?}", time_since_last_speed_check, state.speed_check_interval));
        
        if now.duration_since(last_speed_check) >= state.speed_check_interval {
            log_with_timestamp("Speed check interval reached, performing speed/quality check...");
            if primary_metrics.status == InterfaceStatus::Working && secondary_metrics.status == InterfaceStatus::Working {
                log_with_timestamp("Both interfaces working, running detailed latency measurements");
                // Run heavier ping
                log_with_timestamp("Measuring detailed latency on primary interface");
                let (_, p_avg) = measure_latency(&state.primary_iface, &state.peer_ip, 5, 5);
                log_with_timestamp("Measuring detailed latency on secondary interface");
                let (_, s_avg) = measure_latency(&state.secondary_iface, &state.peer_ip, 5, 5);
                
                primary_metrics.speed_latency_ms = p_avg;
                secondary_metrics.speed_latency_ms = s_avg;
                
                info!("Speed/Latency Result - {}: {:.1}ms, {}: {:.1}ms", 
                     state.primary_iface, p_avg, state.secondary_iface, s_avg);
                log_with_timestamp("Speed metrics updated successfully");
            } else {
                log_with_timestamp("Skipping speed test because at least one interface is not working");
            }
            last_speed_check = now;
            log_with_timestamp(&format!("Last speed check time updated to: {:?}", last_speed_check));
        } else {
            log_with_timestamp("Speed check not due yet, skipping");
        }

        // ----------------------------------------
        // 4. Decision Logic
        // ----------------------------------------
        log_with_timestamp("Starting decision logic for interface selection");
        let target_interface = match (&primary_metrics.status, &secondary_metrics.status) {
            (InterfaceStatus::Working, InterfaceStatus::Failed) => {
                log_with_timestamp("Decision: Primary works, secondary fails -> Selecting Primary");
                // Primary works, secondary fails -> Primary
                Some((&state.primary_iface, &primary_gw))
            },
            (InterfaceStatus::Failed, InterfaceStatus::Working) => {
                log_with_timestamp("Decision: Primary fails, secondary works -> Selecting Secondary");
                // Primary fails, secondary works -> Secondary
                Some((&state.secondary_iface, &secondary_gw))
            },
            (InterfaceStatus::Working, InterfaceStatus::Working) => {
                log_with_timestamp("Decision: Both interfaces working, evaluating speed");
                // Both work. Check preference and speed.
                // Default is primary.
                // If secondary is significantly faster (lower latency), switch.
                // Note: "Faster" here uses latency as proxy. Lower is better.
                
                let p_lat = primary_metrics.speed_latency_ms;
                let s_lat = secondary_metrics.speed_latency_ms;
                log_with_timestamp(&format!("Speed latencies - Primary: {:.1}ms, Secondary: {:.1}ms", p_lat, s_lat));
                
                // If we are currently on Primary, only switch if Secondary is MUCH better (lower latency)
                // Threshold is percentage.
                // If Secondary is < Primary * (1 - threshold/100)
                let threshold_factor = 1.0 - (state.speed_threshold as f64 / 100.0);
                log_with_timestamp(&format!("Speed threshold factor: {:.2} (threshold: {}%)", threshold_factor, state.speed_threshold));
                
                if s_lat > 0.0 && p_lat > 0.0 && s_lat < (p_lat * threshold_factor) {
                    log_with_timestamp(&format!("Secondary is significantly faster ({} < {} * {}), switching to Secondary", s_lat, p_lat, threshold_factor));
                    info!("Secondary {} ({:.1}ms) is significantly faster than Primary {} ({:.1}ms). Switching.", 
                          state.secondary_iface, s_lat, state.primary_iface, p_lat);
                    Some((&state.secondary_iface, &secondary_gw))
                } else {
                    log_with_timestamp("Secondary not significantly faster or speed data unavailable, sticking with Primary");
                    // Stick with Primary usually
                    Some((&state.primary_iface, &primary_gw))
                }
            },
            (InterfaceStatus::Failed, InterfaceStatus::Failed) => {
                log_with_timestamp("Decision: Both interfaces failed");
                warn!("Both interfaces failed connectivity check.");
                // Keep existing or do nothing?
                // If we do nothing, we stay on the last set route.
                None
            },
            _ => {
                log_with_timestamp("Decision: Unknown interface status combination, no target selected");
                None
            }
        };
        log_with_timestamp(&format!("Decision result: target_interface = {:?}", target_interface));

        // ----------------------------------------
        // 5. Apply Route Change
        // ----------------------------------------
        log_with_timestamp("Evaluating route changes");
        if let Some((target_iface, target_gw)) = target_interface {
            log_with_timestamp(&format!("Target interface selected: {}, gateway: {:?}", target_iface, target_gw));
            let should_update = match &current_active_interface {
                Some(current) => {
                    let update_needed = current != target_iface;
                    log_with_timestamp(&format!("Current active interface: {}, update needed: {}", current, update_needed));
                    update_needed // Changed interface
                },
                None => {
                    log_with_timestamp("No current active interface (first run), update needed");
                    true // First run
                },
            };

            // Force update if it's been a while? Or just on change.
            // Also need to handle if gateway changed.
            // For simplicity, we update if interface changed OR if we just want to ensure consistency.
            // Let's only update on change to avoid log spam, but maybe retry if previous attempt failed?
            
            if should_update {
                log_with_timestamp(&format!("Routing WireGuard Peer {} via {}", state.peer_ip, target_iface));
                match update_route(&state.peer_ip, target_iface, target_gw.as_ref()) {
                    Ok(_) => {
                        current_active_interface = Some(target_iface.clone());
                        log_with_timestamp("Route updated successfully.");
                    },
                    Err(e) => {
                        error!("Failed to update route: {}", e);
                        log_with_timestamp(&format!("Route update failed with error: {}", e));
                    }
                }
            } else {
                log_with_timestamp("No route change needed, interface unchanged");
            }
        } else {
            log_with_timestamp("No target interface selected, skipping route update");
        }

        // Sleep
        log_with_timestamp(&format!("Sleeping for {:?} before next iteration", state.check_interval));
        thread::sleep(state.check_interval);
        log_with_timestamp("Awake from sleep, starting next loop iteration");
    }
}