//! MQTT connection running on a background thread.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use crossbeam_channel::{Receiver, Sender, unbounded};
use rumqttc::{Client, Event, MqttOptions, Packet, QoS};

use crate::Notify;
use crate::config::BrokerConfig;

/// Topics subscribed on every (re)connection.
/// Answers first: LWT and discovery messages make the app ask for a device's status, and
/// the answer must not arrive before the app listens to stat/ (old Tasmota answers in
/// separate messages, some of which would be lost).
const BASE_TOPICS: &[&str] = &[
    "stat/+/+",
    "tele/+/+",
    "+/+/LWT",
    "tasmota/discovery/+/config",
];

#[derive(Debug)]
pub enum MqttEvent {
    Connected,
    Disconnected(String),
    /// `retained`: replayed by the broker on subscribe, not sent just now.
    Message {
        topic: String,
        payload: String,
        retained: bool,
    },
}

pub struct Mqtt {
    client: Client,
    stop: Arc<AtomicBool>,
    extra_topics: Arc<Mutex<Vec<String>>>,
}

impl Mqtt {
    /// Starts the connection thread. Events arrive on the returned receiver.
    pub fn start(cfg: &BrokerConfig, notify: Notify) -> (Mqtt, Receiver<MqttEvent>) {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.subsec_nanos())
            .unwrap_or(0);
        let client_id = format!("tasmota-derust-{:x}{:x}", std::process::id(), nonce);

        let mut opts = MqttOptions::new(client_id, cfg.host.trim(), cfg.port);
        opts.set_keep_alive(Duration::from_secs(30));
        opts.set_clean_session(true);
        opts.set_max_packet_size(512 * 1024, 64 * 1024);
        if !cfg.username.is_empty() {
            opts.set_credentials(cfg.username.clone(), cfg.password.clone());
        }

        let (client, mut connection) = Client::new(opts, 256);
        let (tx, rx) = unbounded();
        let stop = Arc::new(AtomicBool::new(false));
        let extra_topics = Arc::new(Mutex::new(Vec::new()));

        let thread_client = client.clone();
        let thread_stop = stop.clone();
        let thread_extra = extra_topics.clone();
        thread::Builder::new()
            .name("mqtt".into())
            .spawn(move || {
                run(
                    &mut connection,
                    &thread_client,
                    &thread_stop,
                    &thread_extra,
                    &tx,
                    &notify,
                )
            })
            .expect("spawn mqtt thread");

        (
            Mqtt {
                client,
                stop,
                extra_topics,
            },
            rx,
        )
    }

    pub fn publish(&self, topic: &str, payload: &str) {
        let _ = self
            .client
            .try_publish(topic, QoS::AtMostOnce, false, payload.as_bytes().to_vec());
    }

    /// Adds a subscription that is also restored after reconnects.
    pub fn subscribe(&self, topic: &str) {
        let mut extra = self.extra_topics.lock().unwrap();
        if !extra.iter().any(|t| t == topic) {
            extra.push(topic.to_string());
            let _ = self.client.try_subscribe(topic, QoS::AtMostOnce);
        }
    }
}

impl Drop for Mqtt {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        let _ = self.client.try_disconnect();
    }
}

fn run(
    connection: &mut rumqttc::Connection,
    client: &Client,
    stop: &AtomicBool,
    extra: &Mutex<Vec<String>>,
    tx: &Sender<MqttEvent>,
    notify: &Notify,
) {
    for event in connection.iter() {
        if stop.load(Ordering::SeqCst) {
            break;
        }
        let out = match event {
            Ok(Event::Incoming(Packet::ConnAck(_))) => {
                for t in BASE_TOPICS {
                    let _ = client.try_subscribe(*t, QoS::AtMostOnce);
                }
                for t in extra.lock().unwrap().iter() {
                    let _ = client.try_subscribe(t.clone(), QoS::AtMostOnce);
                }
                Some(MqttEvent::Connected)
            }
            Ok(Event::Incoming(Packet::Publish(p))) => Some(MqttEvent::Message {
                retained: p.retain,
                topic: p.topic,
                payload: String::from_utf8_lossy(&p.payload).into_owned(),
            }),
            Ok(_) => None,
            Err(e) => {
                let _ = tx.send(MqttEvent::Disconnected(e.to_string()));
                notify();
                // Avoid a busy loop while the broker is unreachable.
                for _ in 0..30 {
                    if stop.load(Ordering::SeqCst) {
                        return;
                    }
                    thread::sleep(Duration::from_millis(100));
                }
                None
            }
        };
        if let Some(ev) = out {
            if tx.send(ev).is_err() {
                break;
            }
            notify();
        }
    }
}
