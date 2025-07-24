//! # WireGuard Failover
//! 
//! A network failover utility for maintaining continuous WireGuard VPN connectivity
//! by monitoring multiple network interfaces and automatically switching between them
//! when connectivity issues are detected.
//!
//! This library provides functions for monitoring network connectivity and
//! managing routing to ensure uninterrupted VPN connections.

pub mod errors;
pub mod network;

// Re-export commonly used types and functions
pub use errors::{FailoverError, FailoverResult};
pub use network::{
    get_current_interface,
    get_gateway_for_interface,
    interface_exists,
    list_interfaces,
    ping_interface,
    switch_interface,
    tcp_connection_test,
    get_wifi_signal_strength,
    is_wireless_interface,
    get_interface_addresses,
};

/// Network status representing the current active interface
#[derive(Debug, Clone, PartialEq)]
pub enum NetworkStatus {
    /// Primary interface is active
    Primary,
    
    /// Secondary interface is active
    Secondary,
    
    /// No interface is able to reach the target
    Unavailable,
}

/// Configuration for the failover monitor
#[derive(Debug, Clone)]
pub struct FailoverConfig {
    /// The IP address or hostname of the WireGuard peer
    pub peer_ip: String,
    
    /// The WireGuard interface name (e.g., wg0)
    pub wg_interface: String,
    
    /// Primary network interface (e.g., eth0, enp0s31f6)
    pub primary_interface: String,
    
    /// Secondary network interface (e.g., wlan0, wlp0s20f0u5)
    pub secondary_interface: String,
    
    /// Ping interval in seconds
    pub check_interval: u64,
    
    /// Number of ping attempts
    pub ping_count: u8,
    
    /// Ping timeout in seconds
    pub ping_timeout: u8,
}

impl Default for FailoverConfig {
    fn default() -> Self {
        FailoverConfig {
            peer_ip: String::new(),
            wg_interface: "wg0".to_string(),
            primary_interface: String::new(),
            secondary_interface: String::new(),
            check_interval: 30,
            ping_count: 2,
            ping_timeout: 2,
        }
    }
}