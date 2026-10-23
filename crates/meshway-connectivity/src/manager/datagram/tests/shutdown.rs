//! A socket owner's shutdown against outputs its callers keep unpolled.
use super::*;

/// U4 (fault/throttle, alpha.23: B's packet plane never finished its
/// withdrawal, Status `retrying · dns` with `retryAt` null): an output
/// first polled while another admission held the shared sender, then kept
/// unpolled (as the packet owner keeps its sends while it reconfigures),
/// must not keep that sender from the owner's shutdown, which the same
/// reconfiguration joins. Red before: the queued lock waiter was handed the
/// lock when the holder released it and kept it unpolled; the shutdown,
/// locking the same sender, never ended.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_output_kept_unpolled_does_not_hold_the_sender_from_shutdown() {
    let mut fixture = Netmaps::start(false, true).await;
    let ids = fixture.ids();
    let (shared, _events) = shared(&fixture.desired[1], &fixture.keys[1]);
    let mut owner = accepted_link(&mut fixture, &shared)
        .await
        .downcast::<Owner>()
        .expect("the socket owner");
    let (scope, remote) = {
        let links = shared.links.lock().unwrap();
        let link = &links[&ids[0]];
        (link.scope.clone(), link.fact.remote)
    };
    let sender = owner.sender.clone();
    // Another admission (a receive owner's reply, on another thread) holds
    // the sender while the output is first polled.
    let (held, holding) = std::sync::mpsc::channel();
    let holder = std::thread::spawn({
        let sender = sender.clone();
        move || {
            let guard = sender.lock().unwrap();
            held.send(()).unwrap();
            std::thread::sleep(Duration::from_millis(100));
            drop(guard);
        }
    });
    holding.recv().unwrap();
    let mut output = Box::pin(send_control(&sender, &scope, remote, vec![0; 8]));
    let _ = futures_util::poll!(&mut output);
    holder.join().unwrap();
    // The output stays alive and is not polled again.
    tokio::time::timeout(Duration::from_secs(2), owner.shutdown())
        .await
        .expect("the kept output does not hold the sender")
        .unwrap();
    drop(output);
    fixture.close().await;
}
