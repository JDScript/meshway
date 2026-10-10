use super::*;

pub(super) fn identity_rejection() -> ClientError<meshway_protocol::ErrorResponse> {
    ClientError::Http {
        status: 403,
        problem: Some(meshway_protocol::ErrorResponse {
            request_id: None,
            status: 403,
            problem: meshway_protocol::Problem::NodeIdentityRejected {
                params: meshway_protocol::EmptyParams {},
                message: "identity rejected".into(),
            },
        }),
    }
}

#[tokio::test]
async fn intermediary_status_and_resource_rejection_preserve_signed_authority() {
    let server = Server::start().await;
    let proxy = Proxy::start(&server.origin).await;
    let (mut control, binding, script) = enrolled(&server, &proxy, "http-rejections").await;
    let netmap = compiled(&binding, "http-rejections", None);
    script.push(Ok(full(&binding, &netmap, HOUR)));
    let held = control.sync(&mut Facts::default()).await.unwrap().snapshot;
    for status in [401, 403, 404] {
        for error in [
            ClientError::Network {
                status: Some(status),
            },
            ClientError::Http {
                status,
                problem: None,
            },
        ] {
            script.push(Err(error));
            assert!(control.sync(&mut Facts::default()).await.is_err());
            held.ensure_current().unwrap();
            assert!(control.netmap_snapshot().unwrap().same_netmap(&held));
            control.authenticate().await.unwrap();
        }
    }
    script.push(Err(ClientError::Http {
        status: 404,
        problem: Some(meshway_protocol::ErrorResponse {
            request_id: None,
            status: 404,
            problem: meshway_protocol::Problem::NotFound {
                params: meshway_protocol::EmptyParams {},
                message: "resource not found".into(),
            },
        }),
    }));
    assert!(control.sync(&mut Facts::default()).await.is_err());
    held.ensure_current().unwrap();
    control.authenticate().await.unwrap();
    script.push(Err(identity_rejection()));
    assert!(control.sync(&mut Facts::default()).await.is_err());
    assert!(held.ensure_current().is_err());
    control.shutdown().await.unwrap();
    server.close().await;
}

#[tokio::test]
async fn stale_signed_same_serial_lease_does_not_reissue_or_cancel_live_guards() {
    let server = Server::start().await;
    let proxy = Proxy::start(&server.origin).await;
    let (mut control, binding, script) = enrolled(&server, &proxy, "stale-lease").await;
    let netmap = compiled(&binding, "stale-lease", None);
    script.push(Ok(full(&binding, &netmap, HOUR)));
    let held = control.sync(&mut Facts::default()).await.unwrap().snapshot;
    let before = held.lease();
    let mut stale = before;
    stale.issued_at_ms -= 1;
    script.push(Ok(signed(&binding, stale, SyncNetmap::Unchanged)));
    assert!(matches!(
        control.sync(&mut Facts::default()).await,
        Err(Error::Contract)
    ));
    assert_eq!(held.lease(), before);
    assert!(control.netmap_snapshot().unwrap().same_netmap(&held));
    held.ensure_current().unwrap();
    control.shutdown().await.unwrap();
    server.close().await;
}
