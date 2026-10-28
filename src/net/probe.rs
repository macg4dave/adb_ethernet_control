//! Interface-bound IPv4 reachability probe (ping).

use std::net::Ipv4Addr;
use std::time::Duration;

use super::interface::InterfaceName;

/// `ping` arguments for one IPv4 echo bound to `interface`, so another
/// uplink cannot hide a dead phone WAN.
pub fn ping_args(interface: &InterfaceName, host: Ipv4Addr, timeout: Duration) -> Vec<String> {
    vec![
        "-4".into(),
        "-n".into(),
        "-q".into(),
        "-c".into(),
        "1".into(),
        "-W".into(),
        timeout.as_secs().max(1).to_string(),
        "-I".into(),
        interface.as_str().into(),
        host.to_string(),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ping_is_bound_and_bounded() {
        let iface = InterfaceName::try_from("usb0".to_owned()).unwrap();
        let args = ping_args(&iface, Ipv4Addr::new(8, 8, 8, 8), Duration::from_secs(5));
        assert_eq!(args.join(" "), "-4 -n -q -c 1 -W 5 -I usb0 8.8.8.8");
    }
}
