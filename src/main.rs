use anyhow::{Context, Result};
use clap::Parser;
use log::{debug, error, info, warn};
use serde::Deserialize;
use std::collections::HashMap;
use std::path::PathBuf;
use std::process::Command;
use std::thread;
use std::time::{Duration, Instant};

#[derive(Parser, Debug)]
#[command(author, version, about, long_about = None)]
struct Args {
    /// IP address or hostname of the WireGuard peer to monitor
    #[arg(short = 'i', long = "peer-ip")]
    peer_ip: Option<String>,

    /// Path to configuration file
    #[arg(short = 'c', long = "config")]
    config: Option<PathBuf>,

    /// Primary network interface (e.g., eth0)
    #[arg(short = 'p', long = "primary")]
    primary: Option<String>,

    /// Secondary network interface (e.g., wlan0)
    #[arg(short = 's', long = "secondary")]
    secondary: Option<String>,

    /// Connectivity check interval in seconds
    #[arg(short = 't', long = "interval")]
    interval: Option<u64>,

    /// Speed test interval in seconds
    #[arg(long = "speedtest-interval")]
    speedtest_interval: Option<u64>,

    /// Speed threshold percentage to switch to faster interface
    #[arg(long = "speed-threshold")]
    speed_threshold: Option<u8>,

    /// Test IPs for connectivity checks (comma-separated)
    #[arg(long = "test-ips")]
    test_ips: Option<String>,

    /// Route all traffic through selected interface (not just WireGuard peer)
    #[arg(long = "route-all-traffic")]
    route_all_traffic: bool,
}

#[derive(Debug, Deserialize)]
struct Config {
    peer: Option<PeerConfig>,
    interfaces: Option<InterfaceConfig>,
    monitoring: Option<MonitoringConfig>,
    test_ips: Option<Vec<String>>,
    route_all_traffic: Option<bool>,
}

#[derive(Debug, Deserialize)]
struct PeerConfig {
    ip: Option<String>,
}

#[derive(Debug, Deserialize)]
struct InterfaceConfig {
    primary: Option<String>,
    secondary: Option<String>,
}

#[derive(Debug, Deserialize)]
struct MonitoringConfig {
    interval: Option<u64>,
    speedtest_interval: Option<u64>,
    speed_threshold: Option<u8>,
}

struct AppState {
    peer_ip: String,
    primary_iface: String,
    secondary_iface: String,
    test_ips: Vec<String>,
    check_interval: Duration,
    speed_check_interval: Duration,
    speed_threshold: u8,
    route_all_traffic: bool,
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
    test_results: HashMap<String, bool>, // IP -> reachable
}

impl Default for InterfaceMetrics {
    fn default() -> Self {
        Self {
            status: InterfaceStatus::Unknown,
            connectivity_latency_ms: 0.0,
            speed_latency_ms: 0.0,
            test_results: HashMap::new(),
        }
    }
}

fn log_with_timestamp(msg: &str) {
    debug!("[{}] {}", chrono::Local::now().format("%Y-%m-%d %H:%M:%S"), msg);
}

fn get_gateway_for_interface(iface: &str) -> Option<String> {
    debug!("Getting gateway for interface: {}", iface);
    
    // Try to get default gateway for this interface
    let output = Command::new("ip")
        .args(["route", "show", "dev", iface])
        .output();

    match output {
        Ok(out) if out.status.success() => {
            let stdout = String::from_utf8_lossy(&out.stdout);
            debug!("Route output for {}: {}", iface, stdout);
            
            // Look for default route via gateway
            for line in stdout.lines() {
                if line.starts_with("default via ") {
                    let parts: Vec<&str> = line.split_whitespace().collect();
                    if parts.len() >= 3 {
                        debug!("Found default gateway for {}: {}", iface, parts[2]);
                        return Some(parts[2].to_string());
                    }
                }
            }
            
            // If no default route, look for any route with a gateway
            for line in stdout.lines() {
                if line.contains(" via ") {
                    let parts: Vec<&str> = line.split_whitespace().collect();
                    for (i, part) in parts.iter().enumerate() {
                        if *part == "via" && i + 1 < parts.len() {
                            debug!("Found gateway for {}: {}", iface, parts[i + 1]);
                            return Some(parts[i + 1].to_string());
                        }
                    }
                }
            }
            
            debug!("No gateway found for interface {}", iface);
            None
        }
        Ok(out) => {
            let stderr = String::from_utf8_lossy(&out.stderr);
            debug!("Failed to get routes for {}: {}", iface, stderr);
            None
        }
        Err(e) => {
            debug!("Failed to execute ip command for {}: {}", iface, e);
            None
        }
    }
}

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

fn test_connectivity_multiple_ips(iface: &str, test_ips: &[String]) -> (bool, f64, HashMap<String, bool>) {
    debug!("Testing connectivity for interface {} to {} IPs", iface, test_ips.len());
    
    let mut successful_tests = 0;
    let mut total_latency = 0.0;
    let mut test_results = HashMap::new();
    
    for ip in test_ips {
        debug!("Testing connectivity to {} via {}", ip, iface);
        let (success, latency) = measure_latency(iface, ip, 1, 2);
        test_results.insert(ip.clone(), success);
        
        if success {
            successful_tests += 1;
            total_latency += latency;
            debug!("Successfully reached {} via {} with latency {:.1}ms", ip, iface, latency);
        } else {
            debug!("Failed to reach {} via {}", ip, iface);
        }
    }
    
    let avg_latency = if successful_tests > 0 {
        total_latency / successful_tests as f64
    } else {
        0.0
    };
    
    // Consider interface working if at least 50% of tests succeed
    let interface_working = successful_tests > 0 && (successful_tests as f32 / test_ips.len() as f32) >= 0.5;
    
    debug!("Interface {}: {} successful tests out of {}, average latency: {:.1}ms, working: {}", 
           iface, successful_tests, test_ips.len(), avg_latency, interface_working);
    
    (interface_working, avg_latency, test_results)
}

fn update_route_for_peer(peer_ip: &str, iface: &str, gateway: Option<&String>) -> Result<()> {
    debug!("update_route_for_peer called: peer_ip={}, iface={}, gateway={:?}", peer_ip, iface, gateway);
    
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

fn update_default_route(iface: &str, gateway: Option<&String>) -> Result<()> {
    debug!("update_default_route called: iface={}, gateway={:?}", iface, gateway);
    
    // Command: ip route replace default [via <gateway>] dev <iface>
    let mut cmd = Command::new("ip");
    cmd.arg("route").arg("replace").arg("default");
    
    if let Some(gw) = gateway {
        debug!("Adding gateway to default route: via {}", gw);
        cmd.arg("via").arg(gw);
    } else {
        debug!("No gateway specified for default route");
    }
    
    cmd.arg("dev").arg(iface);
    cmd.arg("metric").arg("100");
    
    let cmd_str = format!("{:?}", cmd);
    debug!("Executing default route command: {}", cmd_str);

    let output = cmd.output().context("Failed to execute ip route command")?;
    
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let stdout = String::from_utf8_lossy(&output.stdout);
        debug!("Default route command failed with status: {}", output.status);
        debug!("Default route command stderr: {}", stderr);
        debug!("Default route command stdout: {}", stdout);
        return Err(anyhow::anyhow!("ip route default failed: {}", stderr));
    }
    
    debug!("Default route command succeeded with status: {}", output.status);
    let stdout = String::from_utf8_lossy(&output.stdout);
    if !stdout.is_empty() {
        debug!("Default route command stdout: {}", stdout);
    }
    let stderr = String::from_utf8_lossy(&output.stderr);
    if !stderr.is_empty() {
        debug!("Default route command stderr: {}", stderr);
    }
    
    debug!("Updated default route via {} (gw: {:?})", iface, gateway);
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

    // Get test IPs from args or config, default to common public DNS servers
    let test_ips = if let Some(ips_str) = args.test_ips {
        ips_str.split(',').map(|s| s.trim().to_string()).collect()
    } else if let Some(ips) = config_file.as_ref().and_then(|c| c.test_ips.as_ref()) {
        ips.clone()
    } else {
        // Default test IPs: common public DNS servers
        vec![
            "8.8.8.8".to_string(),      // Google DNS
            "1.1.1.1".to_string(),      // Cloudflare DNS
            "208.67.222.222".to_string(), // OpenDNS
            peer_ip.clone(),             // Include the WireGuard peer
        ]
    };
    log_with_timestamp(&format!("Test IPs determined: {:?}", test_ips));

    let route_all_traffic = args.route_all_traffic
        || config_file.as_ref().and_then(|c| c.route_all_traffic).unwrap_or(false);
    log_with_timestamp(&format!("Route all traffic: {}", route_all_traffic));

    log_with_timestamp("Creating application state");
    let state = AppState {
        peer_ip,
        primary_iface,
        secondary_iface,
        test_ips,
        check_interval: Duration::from_secs(interval_secs),
        speed_check_interval: Duration::from_secs(speed_interval_secs),
        speed_threshold,
        route_all_traffic,
    };
    log_with_timestamp("Application state created successfully");

    log_with_timestamp("Starting WireGuard Failover (Multiple IP Test Mode)");
    info!("Peer: {}", state.peer_ip);
    info!("Primary: {}, Secondary: {}", state.primary_iface, state.secondary_iface);
    info!("Test IPs: {:?}", state.test_ips);
    info!("Route all traffic: {}", state.route_all_traffic);
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
        // 2. Connectivity Check (Frequent) - Multiple IPs
        // ----------------------------------------
        log_with_timestamp("Starting connectivity checks with multiple IPs");
        log_with_timestamp(&format!("Checking connectivity via primary interface: {}", state.primary_iface));
        let (p_ok, p_lat, p_results) = test_connectivity_multiple_ips(&state.primary_iface, &state.test_ips);
        log_with_timestamp(&format!("Primary interface connectivity result: success={}, average latency={:.1}ms", p_ok, p_lat));
        
        log_with_timestamp(&format!("Checking connectivity via secondary interface: {}", state.secondary_iface));
        let (s_ok, s_lat, s_results) = test_connectivity_multiple_ips(&state.secondary_iface, &state.test_ips);
        log_with_timestamp(&format!("Secondary interface connectivity result: success={}, average latency={:.1}ms", s_ok, s_lat));

        log_with_timestamp("Updating metrics based on connectivity results");
        primary_metrics.status = if p_ok { InterfaceStatus::Working } else { InterfaceStatus::Failed };
        primary_metrics.connectivity_latency_ms = p_lat;
        primary_metrics.test_results = p_results;
        log_with_timestamp(&format!("Primary metrics updated: status={:?}, latency={:.1}ms", primary_metrics.status, primary_metrics.connectivity_latency_ms));
        
        secondary_metrics.status = if s_ok { InterfaceStatus::Working } else { InterfaceStatus::Failed };
        secondary_metrics.connectivity_latency_ms = s_lat;
        secondary_metrics.test_results = s_results;
        log_with_timestamp(&format!("Secondary metrics updated: status={:?}, latency={:.1}ms", secondary_metrics.status, secondary_metrics.connectivity_latency_ms));

        // Log detailed test results
        for (ip, p_reachable) in &primary_metrics.test_results {
            let s_reachable = secondary_metrics.test_results.get(ip).unwrap_or(&false);
            debug!("IP {}: Primary={}, Secondary={}", ip, p_reachable, s_reachable);
        }

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
                // Run heavier ping to peer IP for speed comparison
                log_with_timestamp("Measuring detailed latency on primary interface to peer");
                let (_, p_avg) = measure_latency(&state.primary_iface, &state.peer_ip, 5, 5);
                log_with_timestamp("Measuring detailed latency on secondary interface to peer");
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

            if should_update {
                if state.route_all_traffic {
                    log_with_timestamp(&format!("Routing ALL traffic via {}", target_iface));
                    match update_default_route(target_iface, target_gw.as_ref()) {
                        Ok(_) => {
                            current_active_interface = Some(target_iface.clone());
                            log_with_timestamp("Default route updated successfully.");
                        },
                        Err(e) => {
                            error!("Failed to update default route: {}", e);
                            log_with_timestamp(&format!("Default route update failed with error: {}", e));
                        }
                    }
                } else {
                    log_with_timestamp(&format!("Routing WireGuard Peer {} via {}", state.peer_ip, target_iface));
                    match update_route_for_peer(&state.peer_ip, target_iface, target_gw.as_ref()) {
                        Ok(_) => {
                            current_active_interface = Some(target_iface.clone());
                            log_with_timestamp("Peer route updated successfully.");
                        },
                        Err(e) => {
                            error!("Failed to update peer route: {}", e);
                            log_with_timestamp(&format!("Peer route update failed with error: {}", e));
                        }
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