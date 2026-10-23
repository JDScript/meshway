//! U2b: an authentication of the sync loop superseded by another one while
//! its request ran is no sync failure.
use super::{
    super::{
        Server,
        faults::{Fault, Proxy},
    },
    HOUR, enrolled,
    support::{Facts, compiled, full},
};
use meshway_control::sync_loop;
use std::time::Duration;

const SESSION: &str = "/auth/session";

/// The sync loop's re-authentication is held at the Server while another
/// task authenticates (the maintenance cycle): the held answer arrives for a
/// changed context (`Pending`). It changed nothing and failed nothing: the
/// loop publishes no error and syncs on the other session. Before, it was
/// published as `Contract` ("Node sync failed; retrying {error: Contract}").
#[tokio::test]
async fn a_superseded_authentication_of_the_sync_loop_is_no_failure() {
    let server = Server::start().await;
    let proxy = Proxy::start(&server.origin).await;
    let (mut control, binding, script) = enrolled(&server, &proxy, "superseded").await;
    proxy.fault(SESSION, Fault::ShortSession);
    control.refresh_authentication().await.unwrap();
    let netmap = compiled(&binding, "superseded", None);
    script.push(Ok(full(&binding, &netmap, HOUR)));
    // The 2 s session has ended: the loop authenticates first.
    tokio::time::sleep(Duration::from_millis(2500)).await;
    let control = tokio::sync::Mutex::new(control);
    let (sender, mut receiver) =
        tokio::sync::watch::channel::<Option<meshway_engine::Netmap>>(None);
    let (errors, mut failures) = tokio::sync::watch::channel(None);
    let stop = tokio_util::sync::CancellationToken::new();
    proxy.fault(SESSION, Fault::Hold);
    let mut sink = Facts::default();
    // Every class the loop publishes until the netmap arrives (an empty
    // script answers the next long poll `Network` afterwards).
    let mut seen = Vec::new();
    let observe = async {
        tokio::time::timeout(Duration::from_secs(5), proxy.committed())
            .await
            .unwrap();
        // Another task authenticates meanwhile, then the held answer arrives.
        meshway_control::shared::authenticate(&control)
            .await
            .unwrap();
        proxy.release();
        loop {
            tokio::select! { biased;
                changed = receiver.changed() => {
                    changed.unwrap();
                    break;
                }
                changed = failures.changed() => {
                    changed.unwrap();
                    seen.push(*failures.borrow_and_update());
                }
            }
        }
        assert!(receiver.borrow_and_update().is_some());
        stop.cancel();
    };
    let (result, ()) = tokio::time::timeout(Duration::from_secs(15), async {
        tokio::join!(
            sync_loop(&control, &mut sink, &sender, &errors, &stop),
            observe
        )
    })
    .await
    .unwrap();
    result.unwrap();
    assert!(
        seen.is_empty(),
        "a superseded authentication published {seen:?}"
    );
    assert_eq!(
        proxy.count(SESSION),
        4,
        "enrollment's, the short one, the held one and the other task's"
    );
    proxy.close().await;
    control.into_inner().shutdown().await.unwrap();
    server.close().await;
}
