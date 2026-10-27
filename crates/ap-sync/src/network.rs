//! Interface-change watcher; no socket or startup retries while offline.
//!
//! Like Android's `NET_CAPABILITY_INTERNET` check, a usable (non-loopback, non-link-local)
//! interface counts as online. No external reachability probe: sync servers are often
//! LAN-only, and a public-host probe would gate them on unrelated internet access.
use std::{net::IpAddr, time::Duration};
use tokio::sync::watch;

pub fn network_availability() -> watch::Receiver<bool> {
    let (tx, rx) = watch::channel(false);
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(Duration::from_secs(3));
        loop {
            tokio::select! { () = tx.closed() => break, _ = interval.tick() => {} }
            let interfaces = tokio::task::spawn_blocking(if_addrs::get_if_addrs).await;
            let online =
                matches!(&interfaces, Ok(Ok(entries)) if entries.iter().any(|i| usable(i.ip())));
            tx.send_if_modified(|current| {
                let changed = *current != online;
                *current = online;
                changed
            });
        }
    });
    rx
}

fn usable(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => !v4.is_loopback() && !v4.is_link_local() && !v4.is_unspecified(),
        IpAddr::V6(v6) => !v6.is_loopback() && !v6.is_unspecified() && !v6.is_unicast_link_local(),
    }
}

#[cfg(test)]
mod tests {
    use super::usable;

    #[test]
    fn loopback_and_link_local_are_not_connectivity() {
        assert!(!usable("127.0.0.1".parse().unwrap()));
        assert!(!usable("169.254.3.4".parse().unwrap()));
        assert!(!usable("::1".parse().unwrap()));
        assert!(!usable("fe80::1".parse().unwrap()));
        assert!(usable("192.168.1.20".parse().unwrap()));
        assert!(usable("2001:db8::5".parse().unwrap()));
    }
}
