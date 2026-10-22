//! The platform seam for what `glassd` does not own: Wi-Fi and Bluetooth.
//!
//! Real back ends come later: the Android glue on route 1 (milestone 3), connman or iwd and
//! BlueZ on route 2. Until then the launcher uses [`MockPlatform`].

/// The state of a radio.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RadioStatus {
    pub enabled: bool,
    /// Wi-Fi: the network joined. Bluetooth: the connected devices.
    pub connected: Vec<String>,
}

impl RadioStatus {
    pub fn off() -> RadioStatus {
        RadioStatus {
            enabled: false,
            connected: Vec::new(),
        }
    }

    pub fn on(connected: &[&str]) -> RadioStatus {
        RadioStatus {
            enabled: true,
            connected: connected.iter().map(|s| s.to_string()).collect(),
        }
    }
}

/// Wi-Fi and Bluetooth, as far as the settings cards need them.
pub trait Platform {
    /// A short name for logs and the device card ("mock", "android", "connman").
    fn name(&self) -> &'static str;
    fn wifi(&self) -> RadioStatus;
    fn set_wifi(&mut self, enabled: bool) -> Result<(), String>;
    fn bluetooth(&self) -> RadioStatus;
    fn set_bluetooth(&mut self, enabled: bool) -> Result<(), String>;
}

/// An in-memory platform: Wi-Fi on and joined to "HomeNet", Bluetooth on with nothing
/// connected. Turning a radio on again restores what it had.
#[derive(Clone, Debug)]
pub struct MockPlatform {
    wifi: RadioStatus,
    bluetooth: RadioStatus,
    wifi_network: Vec<String>,
    bt_devices: Vec<String>,
    /// Makes every change fail (tests of the error path).
    pub fail: bool,
}

impl Default for MockPlatform {
    fn default() -> MockPlatform {
        MockPlatform {
            wifi: RadioStatus::on(&["HomeNet"]),
            bluetooth: RadioStatus::on(&[]),
            wifi_network: vec!["HomeNet".into()],
            bt_devices: vec![],
            fail: false,
        }
    }
}

impl MockPlatform {
    pub fn new(wifi: RadioStatus, bluetooth: RadioStatus) -> MockPlatform {
        MockPlatform {
            wifi_network: wifi.connected.clone(),
            bt_devices: bluetooth.connected.clone(),
            wifi,
            bluetooth,
            fail: false,
        }
    }
}

fn toggle(r: &mut RadioStatus, remembered: &[String], enabled: bool) {
    r.enabled = enabled;
    r.connected = if enabled {
        remembered.to_vec()
    } else {
        Vec::new()
    };
}

impl Platform for MockPlatform {
    fn name(&self) -> &'static str {
        "mock"
    }

    fn wifi(&self) -> RadioStatus {
        self.wifi.clone()
    }

    fn set_wifi(&mut self, enabled: bool) -> Result<(), String> {
        if self.fail {
            return Err("mock failure".into());
        }
        toggle(&mut self.wifi, &self.wifi_network, enabled);
        Ok(())
    }

    fn bluetooth(&self) -> RadioStatus {
        self.bluetooth.clone()
    }

    fn set_bluetooth(&mut self, enabled: bool) -> Result<(), String> {
        if self.fail {
            return Err("mock failure".into());
        }
        toggle(&mut self.bluetooth, &self.bt_devices, enabled);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mock_radios_turn_off_and_back_on() {
        let mut p = MockPlatform::default();
        assert_eq!(p.wifi(), RadioStatus::on(&["HomeNet"]));
        p.set_wifi(false).unwrap();
        assert_eq!(p.wifi(), RadioStatus::off());
        p.set_wifi(true).unwrap();
        assert_eq!(p.wifi().connected, vec!["HomeNet"]);
        p.set_bluetooth(false).unwrap();
        assert!(!p.bluetooth().enabled);
        p.fail = true;
        assert!(p.set_bluetooth(true).is_err());
        assert!(!p.bluetooth().enabled);
    }
}
