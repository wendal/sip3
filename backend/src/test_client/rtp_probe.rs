use anyhow::Result;
use std::net::SocketAddr;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;
use tokio::net::UdpSocket;
use tokio::sync::Mutex;
use tokio::task::JoinHandle;

#[derive(Clone)]
pub struct RtpProbe {
    socket: Arc<UdpSocket>,
    peer: Arc<Mutex<Option<SocketAddr>>>,
    rx_count: Arc<AtomicUsize>,
}

impl RtpProbe {
    pub async fn bind(addr: &str) -> Result<Self> {
        let socket = Arc::new(UdpSocket::bind(addr).await?);
        Ok(Self {
            socket,
            peer: Arc::new(Mutex::new(None)),
            rx_count: Arc::new(AtomicUsize::new(0)),
        })
    }

    pub fn local_addr(&self) -> SocketAddr {
        self.socket.local_addr().expect("local addr")
    }

    pub async fn set_peer(&self, peer: SocketAddr) {
        *self.peer.lock().await = Some(peer);
    }

    pub fn rx_count(&self) -> usize {
        self.rx_count.load(Ordering::Relaxed)
    }

    pub fn meets_threshold(&self, threshold: usize) -> bool {
        self.rx_count() >= threshold
    }

    pub fn spawn_receiver(&self) -> JoinHandle<()> {
        let socket = self.socket.clone();
        let rx_count = self.rx_count.clone();

        tokio::spawn(async move {
            let mut buf = [0u8; 1500];
            loop {
                if socket.recv_from(&mut buf).await.is_ok() {
                    rx_count.fetch_add(1, Ordering::Relaxed);
                }
            }
        })
    }

    pub async fn send_packets(&self, count: usize, interval: Duration) -> Result<()> {
        let Some(peer) = *self.peer.lock().await else {
            anyhow::bail!("RTP peer not set");
        };

        for seq in 0..count {
            let payload = format!("rtp-packet-{seq}");
            self.socket.send_to(payload.as_bytes(), peer).await?;
            tokio::time::sleep(interval).await;
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[tokio::test]
    async fn probes_exchange_bidirectional_packets() {
        let caller = RtpProbe::bind("127.0.0.1:0").await.expect("bind caller");
        let callee = RtpProbe::bind("127.0.0.1:0").await.expect("bind callee");

        caller.set_peer(callee.local_addr()).await;
        callee.set_peer(caller.local_addr()).await;

        let caller_task = caller.spawn_receiver();
        let callee_task = callee.spawn_receiver();

        caller
            .send_packets(8, Duration::from_millis(5))
            .await
            .expect("caller send");
        callee
            .send_packets(8, Duration::from_millis(5))
            .await
            .expect("callee send");

        tokio::time::sleep(Duration::from_millis(100)).await;

        assert!(caller.rx_count() >= 4, "caller should receive RTP back");
        assert!(callee.rx_count() >= 4, "callee should receive RTP back");
        let caller_rx = caller.rx_count();
        let callee_rx = callee.rx_count();
        assert!(caller.meets_threshold(caller_rx));
        assert!(!caller.meets_threshold(caller_rx + 1));
        assert!(callee.meets_threshold(callee_rx));
        assert!(!callee.meets_threshold(callee_rx + 1));

        caller_task.abort();
        callee_task.abort();
    }

    #[tokio::test]
    async fn send_packets_without_peer_returns_error() {
        let probe = RtpProbe::bind("127.0.0.1:0").await.expect("bind probe");

        let err = probe
            .send_packets(1, Duration::from_millis(5))
            .await
            .expect_err("send without peer should fail");

        assert!(err.to_string().contains("RTP peer not set"));
    }
}
