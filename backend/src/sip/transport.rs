use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::{Arc, Mutex};
use tokio::sync::mpsc;

#[derive(Clone, Default)]
pub struct TransportRegistry {
    streams: Arc<Mutex<HashMap<SocketAddr, mpsc::UnboundedSender<String>>>>,
}

impl TransportRegistry {
    pub fn register(&self, addr: SocketAddr) -> mpsc::UnboundedReceiver<String> {
        let (tx, rx) = mpsc::unbounded_channel();
        let mut streams = self.streams.lock().expect("transport registry poisoned");
        streams.insert(addr, tx);
        rx
    }

    pub fn unregister(&self, addr: SocketAddr) {
        let mut streams = self.streams.lock().expect("transport registry poisoned");
        streams.remove(&addr);
    }

    pub fn contains(&self, addr: SocketAddr) -> bool {
        let streams = self.streams.lock().expect("transport registry poisoned");
        streams.contains_key(&addr)
    }

    pub fn send(&self, addr: SocketAddr, message: String) -> bool {
        let sender = {
            let streams = self.streams.lock().expect("transport registry poisoned");
            streams.get(&addr).cloned()
        };

        sender.map(|tx| tx.send(message).is_ok()).unwrap_or(false)
    }
}
