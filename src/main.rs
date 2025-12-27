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
    // Run: ip route show dev <iface>
    let output = Command::new("ip")
        .args(["route", "show", "dev", iface])
        .output()
        .ok()?;

    let stdout = String::from_utf8_lossy(&output.stdout);
    
    // Look for lines like "default via 192.168.1.1 ..."
    for line in stdout.lines() {
        let parts: Vec<&str> = line.split_whitespace().collect();
        if parts.len() >= 3 && parts[0] == "default" && parts[1] == "via" {
            return Some(parts[2].to_string());
        }
    }
    
    // If no default route specifically for this dev, try main table generic check? 
    // Usually "ip route show dev X" is sufficient for connected interfaces with gateways.
    // If it's a P2P link, gateway might not be needed.
    None
}

/// Execute ping and return stats (success, avg_latency_ms)
fn measure_latency(iface: &str, target: &str, count: u8, timeout: u8) -> (bool, f64) {
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
            let stdout = String::from_utf8_lossy(&out.stdout);
            // Parse rtt min/avg/max/mdev = 1.1/2.2/3.3/0.4 ms
            for line in stdout.lines() {
                if line.contains("min/avg/max") {
                    if let Some(stats) = line.split('=').nth(1) {
                        let parts: Vec<&str> = stats.split('/').collect();
                        if parts.len() >= 2 {
                            if let Ok(avg) = parts[1].trim().parse::<f64>() {
                                return (true, avg);
                            }
                        }
                    }
                }
            }
            (true, 0.0) // Success but failed to parse latency?
        }
        _ => (false, 0.0),
    }
}

/// Update the route to the peer IP through the specified interface
fn update_route(peer_ip: &str, iface: &str, gateway: Option<&String>) -> Result<()> {
    // Command: ip route replace <peer_ip> [via <gateway>] dev <iface>
    let mut cmd = Command::new("ip");
    cmd.arg("route").arg("replace").arg(peer_ip);
    
    if let Some(gw) = gateway {
        cmd.arg("via").arg(gw);
    }
    
    cmd.arg("dev").arg(iface);
    
    // We can set a metric if we want, but since we are "replacing", 
    // we effectively choose this interface as the active one for this destination.
    // To be cleaner, we can set metric 100.
    cmd.arg("metric").arg("100");

    let output = cmd.output().context("Failed to execute ip route command")?;
    
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(anyhow::anyhow!("ip route failed: {}", stderr));
    }
    
    debug!("Updated route for {} via {} (gw: {:?})", peer_ip, iface, gateway);
    Ok(())
}

fn main() -> Result<()> {
    env_logger::init();
    
    // 1. Load Configuration
    let args = Args::parse();
    
    let config_path = args.config.clone()
        .unwrap_or_else(|| PathBuf::from("/etc/wg-failover/config.toml"));
        
    let config_file: Option<Config> = if config_path.exists() {
        let content = std::fs::read_to_string(&config_path)
            .context(format!("Failed to read config file {:?}", config_path))?;
        Some(toml::from_str(&content).context("Failed to parse TOML")?)
    } else {
        None
    };

    // Helper to extract config values with precedence: Args -> Config File -> Defaults
    let peer_ip = args.peer_ip.clone()
        .or_else(|| config_file.as_ref().and_then(|c| c.peer.as_ref()).and_then(|p| p.ip.clone()))
        .context("Peer IP is required (in args or config)")?;

    let primary_iface = args.primary.clone()
        .or_else(|| config_file.as_ref().and_then(|c| c.interfaces.as_ref()).and_then(|i| i.primary.clone()))
        .context("Primary interface is required")?;

    let secondary_iface = args.secondary.clone()
        .or_else(|| config_file.as_ref().and_then(|c| c.interfaces.as_ref()).and_then(|i| i.secondary.clone()))
        .context("Secondary interface is required")?;
        
    let interval_secs = args.interval
        .or_else(|| config_file.as_ref().and_then(|c| c.monitoring.as_ref()).and_then(|m| m.interval))
        .unwrap_or(30);

    let speed_interval_secs = args.speedtest_interval
        .or_else(|| config_file.as_ref().and_then(|c| c.monitoring.as_ref()).and_then(|m| m.speedtest_interval))
        .unwrap_or(300);
        
    let speed_threshold = args.speed_threshold
        .or_else(|| config_file.as_ref().and_then(|c| c.monitoring.as_ref()).and_then(|m| m.speed_threshold))
        .unwrap_or(20);

    let state = AppState {
        peer_ip,
        primary_iface,
        secondary_iface,
        check_interval: Duration::from_secs(interval_secs),
        speed_check_interval: Duration::from_secs(speed_interval_secs),
        speed_threshold,
    };

    log_with_timestamp(&format!("Starting WireGuard Failover (Simple Metric Mode)"));
    info!("Peer: {}", state.peer_ip);
    info!("Primary: {}, Secondary: {}", state.primary_iface, state.secondary_iface);
    info!("Intervals - Check: {:?}, Speed: {:?}", state.check_interval, state.speed_check_interval);

    let mut primary_metrics = InterfaceMetrics::default();
    let mut secondary_metrics = InterfaceMetrics::default();
    
    // Force check on start if possible, otherwise start timer now
    let mut last_speed_check = Instant::now()
        .checked_sub(state.speed_check_interval)
        .unwrap_or(Instant::now());
    
    let mut current_active_interface: Option<String> = None;

    loop {
        let now = Instant::now();
        
        // ----------------------------------------
        // 1. Identify Gateways (Dynamic, in case of network changes)
        // ----------------------------------------
        let primary_gw = get_gateway_for_interface(&state.primary_iface);
        let secondary_gw = get_gateway_for_interface(&state.secondary_iface);

        // ----------------------------------------
        // 2. Connectivity Check (Frequent)
        // ----------------------------------------
        // We ping the Peer IP through specific interfaces to test reachability.
        // NOTE: If the interface doesn't have a route to peer, ping -I usually works if gateway is on-link or default exists.
        
        let (p_ok, p_lat) = measure_latency(&state.primary_iface, &state.peer_ip, 1, 2);
        let (s_ok, s_lat) = measure_latency(&state.secondary_iface, &state.peer_ip, 1, 2);

        primary_metrics.status = if p_ok { InterfaceStatus::Working } else { InterfaceStatus::Failed };
        primary_metrics.connectivity_latency_ms = p_lat;
        
        secondary_metrics.status = if s_ok { InterfaceStatus::Working } else { InterfaceStatus::Failed };
        secondary_metrics.connectivity_latency_ms = s_lat;

        // ----------------------------------------
        // 3. Speed Check (Periodic)
        // ----------------------------------------
        // Only if both are working, we might want to run a heavier test to decide best path.
        if now.duration_since(last_speed_check) >= state.speed_check_interval {
            log_with_timestamp("Performing speed/quality check...");
            if primary_metrics.status == InterfaceStatus::Working && secondary_metrics.status == InterfaceStatus::Working {
                // Run heavier ping
                let (_, p_avg) = measure_latency(&state.primary_iface, &state.peer_ip, 5, 5);
                let (_, s_avg) = measure_latency(&state.secondary_iface, &state.peer_ip, 5, 5);
                
                primary_metrics.speed_latency_ms = p_avg;
                secondary_metrics.speed_latency_ms = s_avg;
                
                info!("Speed/Latency Result - {}: {:.1}ms, {}: {:.1}ms", 
                     state.primary_iface, p_avg, state.secondary_iface, s_avg);
            }
            last_speed_check = now;
        }

        // ----------------------------------------
        // 4. Decision Logic
        // ----------------------------------------
        let target_interface = match (&primary_metrics.status, &secondary_metrics.status) {
            (InterfaceStatus::Working, InterfaceStatus::Failed) => {
                // Primary works, secondary fails -> Primary
                Some((&state.primary_iface, &primary_gw))
            },
            (InterfaceStatus::Failed, InterfaceStatus::Working) => {
                // Primary fails, secondary works -> Secondary
                Some((&state.secondary_iface, &secondary_gw))
            },
            (InterfaceStatus::Working, InterfaceStatus::Working) => {
                // Both work. Check preference and speed.
                // Default is primary.
                // If secondary is significantly faster (lower latency), switch.
                // Note: "Faster" here uses latency as proxy. Lower is better.
                
                let p_lat = primary_metrics.speed_latency_ms;
                let s_lat = secondary_metrics.speed_latency_ms;
                
                // If we are currently on Primary, only switch if Secondary is MUCH better (lower latency)
                // Threshold is percentage.
                // If Secondary is < Primary * (1 - threshold/100)
                let threshold_factor = 1.0 - (state.speed_threshold as f64 / 100.0);
                
                if s_lat > 0.0 && p_lat > 0.0 && s_lat < (p_lat * threshold_factor) {
                    info!("Secondary {} ({:.1}ms) is significantly faster than Primary {} ({:.1}ms). Switching.", 
                          state.secondary_iface, s_lat, state.primary_iface, p_lat);
                    Some((&state.secondary_iface, &secondary_gw))
                } else {
                    // Stick with Primary usually
                    Some((&state.primary_iface, &primary_gw))
                }
            },
            (InterfaceStatus::Failed, InterfaceStatus::Failed) => {
                warn!("Both interfaces failed connectivity check.");
                // Keep existing or do nothing?
                // If we do nothing, we stay on the last set route.
                None
            },
            _ => None
        };

        // ----------------------------------------
        // 5. Apply Route Change
        // ----------------------------------------
        if let Some((target_iface, target_gw)) = target_interface {
            let should_update = match &current_active_interface {
                Some(current) => current != target_iface, // Changed interface
                None => true, // First run
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
                    }
                }
            }
        }

        // Sleep
        thread::sleep(state.check_interval);
    }
}