//! What the kernel sees on USB (an lsusb-style view), for diagnosing why ADB
//! cannot reach the phone. Pure formatting; the sysfs scan is in `host.rs`.

/// One USB device (not a hub) as the kernel sees it.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct UsbDevice {
    /// Kernel port path, e.g. `7-2` (bus 7, port 2). Stays the same for the
    /// same physical socket, whatever the device reports.
    pub port: String,
    /// Canonical sysfs path.
    pub path: std::path::PathBuf,
    /// Vendor ID, hex.
    pub vendor: String,
    /// Product ID, hex.
    pub product: String,
    /// Manufacturer string.
    pub manufacturer: String,
    /// Product string.
    pub name: String,
    /// iSerialNumber (may be empty).
    pub serial: String,
    /// Interface class/subclass/protocol triples, e.g. `ff4201`, sorted.
    pub interfaces: Vec<String>,
    /// Whether it exposes an ADB interface (`ff4201`).
    pub adb: bool,
    /// Device node and who may open it, e.g.
    /// `/dev/bus/usb/007/023 owner=0:966 mode=0660`.
    pub node: String,
}

impl UsbDevice {
    /// One lsusb-style line. Serials are shortened to their last 4
    /// characters.
    pub fn describe(&self) -> String {
        format!(
            "port={} {}:{} \"{} {}\" serial={} interfaces={} adb_interface={} node={}",
            self.port,
            self.vendor,
            self.product,
            self.manufacturer,
            self.name,
            short_serial(&self.serial),
            self.interfaces.join(","),
            if self.adb { "yes" } else { "no" },
            self.node,
        )
    }
}

fn short_serial(serial: &str) -> String {
    if serial.is_empty() {
        return "-".into();
    }
    let chars: Vec<char> = serial.chars().collect();
    let tail: String = chars[chars.len().saturating_sub(4)..].iter().collect();
    format!("…{tail}")
}

/// The whole scan on one line, for the journal.
pub fn lsusb(devices: &[UsbDevice]) -> String {
    if devices.is_empty() {
        return "none".into();
    }
    devices
        .iter()
        .map(UsbDevice::describe)
        .collect::<Vec<_>>()
        .join(" | ")
}

/// The device with the phone's serial.
pub fn find_phone<'a>(devices: &'a [UsbDevice], serial: &str) -> Option<&'a UsbDevice> {
    devices.iter().find(|d| d.serial == serial)
}

/// ADB-capable devices that are not the configured phone: the phone may
/// show up with another vendor ID or serial in some USB modes.
pub fn other_adb_devices<'a>(devices: &'a [UsbDevice], serial: &str) -> Vec<&'a UsbDevice> {
    devices
        .iter()
        .filter(|d| d.adb && d.serial != serial)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn phone(product: &str, adb: bool) -> UsbDevice {
        UsbDevice {
            port: "7-2".into(),
            vendor: "339b".into(),
            product: product.into(),
            manufacturer: "HONOR".into(),
            name: "ALI-NX1".into(),
            serial: "FAKE0SERIAL1044".into(),
            interfaces: if adb {
                vec!["ff4201".into()]
            } else {
                vec!["060101".into()]
            },
            adb,
            node: "/dev/bus/usb/007/023 owner=0:966 mode=0660".into(),
            ..UsbDevice::default()
        }
    }

    #[test]
    fn describe_is_one_lsusb_like_line_with_port_and_short_serial() {
        assert_eq!(
            phone("107d", true).describe(),
            "port=7-2 339b:107d \"HONOR ALI-NX1\" serial=…1044 interfaces=ff4201 \
             adb_interface=yes node=/dev/bus/usb/007/023 owner=0:966 mode=0660"
        );
    }

    #[test]
    fn finds_phone_and_other_adb_devices() {
        let mut other = phone("4ee7", true);
        other.vendor = "18d1".into();
        other.serial = "OTHER".into();
        other.port = "3-1".into();
        let devices = vec![phone("107d", false), other];
        assert_eq!(
            find_phone(&devices, "FAKE0SERIAL1044").map(|d| d.product.as_str()),
            Some("107d")
        );
        let others = other_adb_devices(&devices, "FAKE0SERIAL1044");
        assert_eq!(others.len(), 1);
        assert_eq!(others[0].port, "3-1");
        assert!(lsusb(&devices).contains(" | port=3-1 18d1:4ee7"));
        assert_eq!(lsusb(&[]), "none");
    }
}
