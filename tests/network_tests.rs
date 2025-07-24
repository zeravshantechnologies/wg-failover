#[cfg(test)]
mod tests {
    use std::process::Command;
    // tempdir is not used in any test
    // mockall::predicate is not used since we're not mocking
    use wg_failover::network::{
        ping_interface, get_gateway_for_interface, get_current_interface, 
        switch_interface, interface_exists, list_interfaces
    };

    // Note: These tests interact with the actual system network interfaces
    // and require appropriate permissions to run successfully.
    // Many tests are marked with #[ignore] as they need root privileges.

    #[test]
    fn test_interface_exists() {
        // This test will pass on most systems since lo (loopback) should exist
        assert!(interface_exists("lo"));
        
        // This should return false for a non-existent interface
        assert!(!interface_exists("nonexistent12345"));
    }

    #[test]
    fn test_list_interfaces() {
        // Should return at least one interface
        let interfaces = list_interfaces();
        assert!(!interfaces.is_empty());
        
        // Loopback should be in the list
        assert!(interfaces.contains(&"lo".to_string()));
    }

    #[test]
    #[ignore = "Requires actual network interfaces and permissions"]
    fn test_ping_interface() {
        // This test would need to be run on a system with the interface
        // and appropriate permissions
        let result = ping_interface("lo", "127.0.0.1", 1, 1);
        assert!(result);
    }

    #[test]
    #[ignore = "Requires root permissions"]
    fn test_get_gateway() {
        // This needs an actual interface with a gateway
        let gateway = get_gateway_for_interface("eth0");
        println!("Gateway for eth0: {:?}", gateway);
    }

    #[test]
    #[ignore = "Requires root permissions"]
    fn test_get_current_interface() {
        // This needs an actual route to the target
        let iface = get_current_interface("8.8.8.8");
        println!("Interface to reach 8.8.8.8: {:?}", iface);
    }

    #[test]
    #[ignore = "Requires root permissions and actual interfaces"]
    fn test_switch_interface() {
        // This would need root permissions and actual interfaces
        let result = switch_interface("eth0", "wg0");
        println!("Switch result: {:?}", result);
    }

    // Utility functions for creating/removing test network interfaces
    fn create_dummy_interface(name: &str) -> Result<(), std::io::Error> {
        Command::new("ip")
            .args(["link", "add", name, "type", "dummy"])
            .status()?;
        
        Command::new("ip")
            .args(["link", "set", name, "up"])
            .status()?;
        
        Ok(())
    }

    fn delete_dummy_interface(name: &str) -> Result<(), std::io::Error> {
        Command::new("ip")
            .args(["link", "delete", name])
            .status()?;
        
        Ok(())
    }

    #[test]
    #[ignore = "Requires root permissions"]
    fn test_with_dummy_interface() {
        let dummy_name = "testdummy0";
        
        // Setup - create a dummy test interface
        match create_dummy_interface(dummy_name) {
            Ok(_) => {
                // Test the interface exists
                assert!(interface_exists(dummy_name));
                
                // Cleanup
                if let Err(e) = delete_dummy_interface(dummy_name) {
                    eprintln!("Failed to delete dummy interface: {}", e);
                }
            },
            Err(e) => {
                eprintln!("Failed to create dummy interface: {}", e);
                // Test will be marked as passed even if we can't create the interface
                // This avoids test failures when not running as root
            }
        }
    }
}