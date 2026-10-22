//! The platform seam for what `glassd` does not own: Wi-Fi, Bluetooth (with pairing for the
//! setup flow), the "set up" flag, and opening other apps.
//!
//! Back ends: [`MockPlatform`] (tests, host runs), the Android wrapper (`glass-android`,
//! milestone 3: requests go to Java as [`PlatformRequest`] lines, state comes back as
//! [`PlatformEvent`] lines), later connman or iwd and BlueZ on route 2.

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

/// Wi-Fi and Bluetooth, as far as the settings cards and the setup flow need them.
pub trait Platform {
    /// A short name for logs and the device card ("mock", "android", "connman").
    fn name(&self) -> &'static str;
    fn wifi(&self) -> RadioStatus;
    fn set_wifi(&mut self, enabled: bool) -> Result<(), String>;
    fn bluetooth(&self) -> RadioStatus;
    fn set_bluetooth(&mut self, enabled: bool) -> Result<(), String>;
    /// The name this Glass shows to phones over Bluetooth (empty if unknown).
    fn bluetooth_name(&self) -> String {
        String::new()
    }
    /// Makes this Glass visible to phones for `seconds`.
    fn set_discoverable(&mut self, _seconds: u32) -> Result<(), String> {
        Ok(())
    }
    /// Answers the pending pairing request (see [`PlatformEvent::PairingRequest`]).
    fn answer_pairing(&mut self, _accept: bool) -> Result<(), String> {
        Ok(())
    }
    /// Records that the setup flow is done (Android: `device_provisioned`).
    fn set_provisioned(&mut self) -> Result<(), String> {
        Ok(())
    }
    /// Opens another app (Android: its launch intent).
    fn open_app(&mut self, package: &str) -> Result<(), String> {
        Err(format!("cannot open {package} on this platform"))
    }
    /// A state change reported by the back end; updates what the platform reports.
    fn on_event(&mut self, _event: &PlatformEvent) {}
}

/// A state change or request from the platform back end, one per line
/// ([`PlatformEvent::parse`]); fields are separated by tabs.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PlatformEvent {
    /// `wifi<TAB>on|off[<TAB>network]`
    Wifi(RadioStatus),
    /// `bluetooth<TAB>on|off[<TAB>device]...`: the connected devices.
    Bluetooth(RadioStatus),
    /// `bt-name<TAB>name`
    BluetoothName(String),
    /// `pairing<TAB>device<TAB>passkey`: a phone asks to pair (empty passkey: consent only).
    PairingRequest { device: String, passkey: String },
    /// `paired<TAB>device<TAB>ok|failed`: a pairing ended.
    Paired { device: String, ok: bool },
    /// `provisioned<TAB>true|false`: whether the system was set up already.
    Provisioned(bool),
}

fn on_off(v: &str) -> Option<bool> {
    match v {
        "on" | "true" => Some(true),
        "off" | "false" => Some(false),
        _ => None,
    }
}

fn radio(enabled: bool, rest: &[&str]) -> RadioStatus {
    RadioStatus {
        enabled,
        connected: if enabled {
            rest.iter()
                .filter(|s| !s.is_empty())
                .map(|s| s.to_string())
                .collect()
        } else {
            Vec::new()
        },
    }
}

impl PlatformEvent {
    /// Parses one event line; `None` for anything unknown or malformed.
    pub fn parse(line: &str) -> Option<PlatformEvent> {
        let f: Vec<&str> = line.trim_end_matches(['\n', '\r']).split('\t').collect();
        Some(match f.as_slice() {
            ["wifi", v, rest @ ..] => PlatformEvent::Wifi(radio(on_off(v)?, rest)),
            ["bluetooth", v, rest @ ..] => PlatformEvent::Bluetooth(radio(on_off(v)?, rest)),
            ["bt-name", name] => PlatformEvent::BluetoothName(name.to_string()),
            ["pairing", device, passkey] => PlatformEvent::PairingRequest {
                device: device.to_string(),
                passkey: passkey.to_string(),
            },
            ["paired", device, r] => PlatformEvent::Paired {
                device: device.to_string(),
                ok: match *r {
                    "ok" => true,
                    "failed" => false,
                    _ => return None,
                },
            },
            ["provisioned", v] => PlatformEvent::Provisioned(on_off(v)?),
            _ => return None,
        })
    }
}

/// A request to the platform back end, one per line ([`PlatformRequest::line`]).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PlatformRequest {
    Wifi(bool),
    Bluetooth(bool),
    Discoverable(u32),
    AnswerPairing(bool),
    SetProvisioned,
    OpenApp(String),
}

/// A field without the separators of the line format.
fn field(s: &str) -> String {
    s.replace(['\t', '\n', '\r'], " ")
}

impl PlatformRequest {
    /// The line sent to the back end: `wifi<TAB>on|off`, `bluetooth<TAB>on|off`,
    /// `discoverable<TAB>seconds`, `pairing<TAB>accept|reject`, `provisioned`,
    /// `open<TAB>package`.
    pub fn line(&self) -> String {
        let oo = |b: bool| if b { "on" } else { "off" };
        match self {
            PlatformRequest::Wifi(b) => format!("wifi\t{}", oo(*b)),
            PlatformRequest::Bluetooth(b) => format!("bluetooth\t{}", oo(*b)),
            PlatformRequest::Discoverable(s) => format!("discoverable\t{s}"),
            PlatformRequest::AnswerPairing(a) => {
                format!("pairing\t{}", if *a { "accept" } else { "reject" })
            }
            PlatformRequest::SetProvisioned => "provisioned".into(),
            PlatformRequest::OpenApp(p) => format!("open\t{}", field(p)),
        }
    }
}

/// Where a [`LinePlatform`] sends its request lines.
pub type LineSender = Box<dyn FnMut(&str) -> Result<(), String>>;

/// A platform that forwards every request as a line to `send` (the Android wrapper's Java
/// side) and reports the state the back end sent as [`PlatformEvent`]s. Changes take
/// effect when the back end reports them.
pub struct LinePlatform {
    name: &'static str,
    send: LineSender,
    wifi: RadioStatus,
    bluetooth: RadioStatus,
    bt_name: String,
}

impl LinePlatform {
    pub fn new(name: &'static str, send: LineSender) -> LinePlatform {
        LinePlatform {
            name,
            send,
            wifi: RadioStatus::off(),
            bluetooth: RadioStatus::off(),
            bt_name: String::new(),
        }
    }

    fn request(&mut self, r: PlatformRequest) -> Result<(), String> {
        (self.send)(&r.line())
    }
}

impl Platform for LinePlatform {
    fn name(&self) -> &'static str {
        self.name
    }
    fn wifi(&self) -> RadioStatus {
        self.wifi.clone()
    }
    fn set_wifi(&mut self, enabled: bool) -> Result<(), String> {
        self.request(PlatformRequest::Wifi(enabled))
    }
    fn bluetooth(&self) -> RadioStatus {
        self.bluetooth.clone()
    }
    fn set_bluetooth(&mut self, enabled: bool) -> Result<(), String> {
        self.request(PlatformRequest::Bluetooth(enabled))
    }
    fn bluetooth_name(&self) -> String {
        self.bt_name.clone()
    }
    fn set_discoverable(&mut self, seconds: u32) -> Result<(), String> {
        self.request(PlatformRequest::Discoverable(seconds))
    }
    fn answer_pairing(&mut self, accept: bool) -> Result<(), String> {
        self.request(PlatformRequest::AnswerPairing(accept))
    }
    fn set_provisioned(&mut self) -> Result<(), String> {
        self.request(PlatformRequest::SetProvisioned)
    }
    fn open_app(&mut self, package: &str) -> Result<(), String> {
        self.request(PlatformRequest::OpenApp(package.into()))
    }
    fn on_event(&mut self, event: &PlatformEvent) {
        match event {
            PlatformEvent::Wifi(r) => self.wifi = r.clone(),
            PlatformEvent::Bluetooth(r) => self.bluetooth = r.clone(),
            PlatformEvent::BluetoothName(n) => self.bt_name = n.clone(),
            _ => {}
        }
    }
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
    /// What the setup flow asked for, in order (tests).
    pub requests: Vec<PlatformRequest>,
}

impl Default for MockPlatform {
    fn default() -> MockPlatform {
        MockPlatform {
            wifi: RadioStatus::on(&["HomeNet"]),
            bluetooth: RadioStatus::on(&[]),
            wifi_network: vec!["HomeNet".into()],
            bt_devices: vec![],
            fail: false,
            requests: Vec::new(),
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
            requests: Vec::new(),
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

    fn bluetooth_name(&self) -> String {
        "Glass".into()
    }

    fn set_discoverable(&mut self, seconds: u32) -> Result<(), String> {
        self.requests.push(PlatformRequest::Discoverable(seconds));
        Ok(())
    }

    fn answer_pairing(&mut self, accept: bool) -> Result<(), String> {
        self.requests.push(PlatformRequest::AnswerPairing(accept));
        Ok(())
    }

    fn set_provisioned(&mut self) -> Result<(), String> {
        self.requests.push(PlatformRequest::SetProvisioned);
        Ok(())
    }

    fn open_app(&mut self, package: &str) -> Result<(), String> {
        self.requests.push(PlatformRequest::OpenApp(package.into()));
        Ok(())
    }

    fn on_event(&mut self, event: &PlatformEvent) {
        if let PlatformEvent::Bluetooth(r) = event {
            self.bluetooth = r.clone();
        }
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

    #[test]
    fn event_lines_parse() {
        use PlatformEvent as E;
        assert_eq!(
            E::parse("wifi\ton\tHome Net"),
            Some(E::Wifi(RadioStatus::on(&["Home Net"])))
        );
        assert_eq!(
            E::parse("wifi\toff\tstale"),
            Some(E::Wifi(RadioStatus::off()))
        );
        assert_eq!(
            E::parse("bluetooth\ton\tPixel 7\tHeadset\n"),
            Some(E::Bluetooth(RadioStatus::on(&["Pixel 7", "Headset"])))
        );
        assert_eq!(
            E::parse("bluetooth\ton"),
            Some(E::Bluetooth(RadioStatus::on(&[])))
        );
        assert_eq!(
            E::parse("pairing\tPixel 7\t012345"),
            Some(E::PairingRequest {
                device: "Pixel 7".into(),
                passkey: "012345".into()
            })
        );
        assert_eq!(
            E::parse("pairing\tPixel 7\t"),
            Some(E::PairingRequest {
                device: "Pixel 7".into(),
                passkey: String::new()
            })
        );
        assert_eq!(
            E::parse("paired\tPixel 7\tfailed"),
            Some(E::Paired {
                device: "Pixel 7".into(),
                ok: false
            })
        );
        assert_eq!(E::parse("provisioned\ttrue"), Some(E::Provisioned(true)));
        assert_eq!(
            E::parse("bt-name\tGlass XE"),
            Some(E::BluetoothName("Glass XE".into()))
        );
        for bad in [
            "",
            "wifi",
            "wifi\tmaybe",
            "paired\tx\tperhaps",
            "nope\t1",
            "provisioned",
        ] {
            assert_eq!(E::parse(bad), None, "{bad:?}");
        }
    }

    #[test]
    fn request_lines() {
        use PlatformRequest as R;
        assert_eq!(R::Wifi(true).line(), "wifi\ton");
        assert_eq!(R::Bluetooth(false).line(), "bluetooth\toff");
        assert_eq!(R::Discoverable(300).line(), "discoverable\t300");
        assert_eq!(R::AnswerPairing(false).line(), "pairing\treject");
        assert_eq!(R::SetProvisioned.line(), "provisioned");
        assert_eq!(R::OpenApp("a\tb".into()).line(), "open\ta b");
    }

    #[test]
    fn line_platform_forwards_and_follows_events() {
        use std::sync::{Arc, Mutex};
        let sent = Arc::new(Mutex::new(Vec::<String>::new()));
        let s2 = sent.clone();
        let mut p = LinePlatform::new(
            "android",
            Box::new(move |l: &str| {
                s2.lock().unwrap().push(l.to_string());
                Ok(())
            }),
        );
        p.set_wifi(true).unwrap();
        assert!(!p.wifi().enabled, "changes wait for the back end's report");
        p.on_event(&PlatformEvent::Wifi(RadioStatus::on(&["Home"])));
        assert_eq!(p.wifi(), RadioStatus::on(&["Home"]));
        p.on_event(&PlatformEvent::BluetoothName("Glass".into()));
        assert_eq!(p.bluetooth_name(), "Glass");
        p.answer_pairing(true).unwrap();
        assert_eq!(*sent.lock().unwrap(), vec!["wifi\ton", "pairing\taccept"]);
    }
}
