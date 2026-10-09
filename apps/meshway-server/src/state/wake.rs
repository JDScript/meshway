//! Wake-ups for stream subscriptions. A subscription re-projects its view when
//! something that can change it happened: a committed effect for its network,
//! a server-wide change, or a deadline it computed itself (lease, report or
//! session expiry). Nothing polls on a timer.
use meshway_model::NetworkId;
use std::collections::BTreeMap;
use std::sync::Mutex;
use tokio::sync::watch;

#[derive(Default)]
pub struct Wakeups {
    networks: Mutex<BTreeMap<NetworkId, watch::Sender<u64>>>,
    all: watch::Sender<u64>,
    activity: watch::Sender<u64>,
}

impl Wakeups {
    /// Called by the effect sink after COMMIT and by shutdown. `None` wakes
    /// every subscription.
    pub fn bump(&self, network: Option<NetworkId>) {
        self.activity
            .send_modify(|version| *version = version.wrapping_add(1));
        match network {
            Some(network) => {
                let Ok(mut networks) = self.networks.lock() else {
                    return;
                };
                if let Some(sender) = networks.get(&network) {
                    sender.send_modify(|version| *version = version.wrapping_add(1));
                }
                // Networks nobody listens to do not keep a channel.
                networks.retain(|_, sender| sender.receiver_count() > 0);
            }
            None => self
                .all
                .send_modify(|version| *version = version.wrapping_add(1)),
        }
    }

    /// App-owned maintenance follows committed changes across its networks.
    pub(crate) fn activity(&self) -> watch::Receiver<u64> {
        self.activity.subscribe()
    }

    pub fn listen(&self, network: NetworkId) -> Listener {
        let Ok(mut networks) = self.networks.lock() else {
            // A poisoned registry cannot wake anyone; such a listener only
            // ever fires on deadlines.
            let (_, receiver) = watch::channel(0);
            return Listener {
                network: receiver,
                all: self.all.subscribe(),
            };
        };
        let sender = networks
            .entry(network)
            .or_insert_with(|| watch::channel(0).0);
        Listener {
            network: sender.subscribe(),
            all: self.all.subscribe(),
        }
    }
}

/// One subscription's view of the wake-up channels. `mark_seen` before a
/// projection and `changed` after it: a bump that lands in between is not lost,
/// it just resolves `changed` immediately.
#[derive(Clone)]
pub struct Listener {
    network: watch::Receiver<u64>,
    all: watch::Receiver<u64>,
}

impl Listener {
    pub fn mark_seen(&mut self) {
        self.network.borrow_and_update();
        self.all.borrow_and_update();
    }

    pub async fn changed(&mut self) {
        // A dropped sender also ends the wait; the caller re-projects and
        // decides whether the stream is still alive.
        tokio::select! {
            _ = self.network.changed() => {}
            _ = self.all.changed() => {}
        }
    }
}
