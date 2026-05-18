 
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

        caller.send_packets(8, Duration::from_millis(5)).await.expect("caller send");
        callee.send_packets(8, Duration::from_millis(5)).await.expect("callee send");

        tokio::time::sleep(Duration::from_millis(100)).await;

        assert!(caller.rx_count() >= 4, "caller should receive RTP back");
        assert!(callee.rx_count() >= 4, "callee should receive RTP back");
        assert!(caller.meets_threshold(4));
        assert!(callee.meets_threshold(4));

        caller_task.abort();
        callee_task.abort();
    }
}
