//! TCP keep-alive for the RDP socket.
//!
//! While the tab is hidden the client sends Suppress Output and the connection carries no
//! traffic in either direction. A firewall or NAT that forgets idle connections then drops it
//! silently, and the next write only fails after the OS retransmission timeout ("operation
//! timed out"). Keep-alive probes keep the state alive and detect a dead peer early.

use std::io;
use std::time::Duration;

use socket2::{SockRef, TcpKeepalive};
use tokio::net::TcpStream;

/// Idle time before the first probe.
pub(crate) const KEEPALIVE_IDLE: Duration = Duration::from_secs(30);
/// Time between probes.
pub(crate) const KEEPALIVE_INTERVAL: Duration = Duration::from_secs(10);
/// Unanswered probes before the connection is declared dead. Supported on macOS, Linux and
/// Windows 10 1703+; on other platforms the retry count is not set (the interval and idle
/// time are set everywhere socket2 offers them).
#[cfg(any(unix, windows))]
pub(crate) const KEEPALIVE_RETRIES: u32 = 3;

/// Turns keep-alive on with the idle time, interval and (where supported) retry count above.
pub(crate) fn enable(stream: &TcpStream) -> io::Result<()> {
    let params = TcpKeepalive::new()
        .with_time(KEEPALIVE_IDLE)
        .with_interval(KEEPALIVE_INTERVAL);
    #[cfg(any(unix, windows))]
    let params = params.with_retries(KEEPALIVE_RETRIES);
    SockRef::from(stream).set_tcp_keepalive(&params)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::net::TcpListener;

    #[tokio::test]
    async fn enable_turns_keepalive_on_with_the_declared_values() {
        let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
        let addr = listener.local_addr().expect("addr");
        let stream = TcpStream::connect(addr).await.expect("connect");
        assert!(!SockRef::from(&stream).keepalive().expect("read"));

        enable(&stream).expect("enable");

        let sock = SockRef::from(&stream);
        assert!(sock.keepalive().expect("keepalive"));
        #[cfg(unix)]
        {
            assert_eq!(sock.tcp_keepalive_time().expect("time"), KEEPALIVE_IDLE);
            assert_eq!(
                sock.tcp_keepalive_interval().expect("interval"),
                KEEPALIVE_INTERVAL
            );
            assert_eq!(
                sock.tcp_keepalive_retries().expect("retries"),
                KEEPALIVE_RETRIES
            );
        }
    }
}
