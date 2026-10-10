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
    let held = control.sync(&mut Facts::default()).await.unwrap().netmap;
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
            assert_eq!(control.netmap().unwrap().lease, held.lease);
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
    assert_eq!(control.netmap().unwrap().lease, held.lease);
    control.authenticate().await.unwrap();
    script.push(Err(identity_rejection()));
    let Err(rejected) = control.sync(&mut Facts::default()).await else {
        panic!("identity rejection")
    };
    assert_eq!(
        rejected.recovery(),
        meshway_control::Recovery::Withdrawn { identity: true }
    );
    assert!(control.netmap().is_none());
    control.shutdown().await.unwrap();
    server.close().await;
}

#[tokio::test]
async fn a_decoded_netmap_withdrawal_ends_the_lease_and_a_new_netmap_restores_it() {
    let server = Server::start().await;
    let proxy = Proxy::start(&server.origin).await;
    let (mut control, binding, script) = enrolled(&server, &proxy, "withdrawn").await;
    let netmap = compiled(&binding, "withdrawn", None);
    script.push(Ok(full(&binding, &netmap, HOUR)));
    control.sync(&mut Facts::default()).await.unwrap();
    script.push(Err(ClientError::Http {
        status: 403,
        problem: Some(meshway_protocol::ErrorResponse {
            request_id: None,
            status: 403,
            problem: meshway_protocol::Problem::NetmapWithdrawn {
                params: meshway_protocol::EmptyParams {},
                message: "withdrawn".into(),
            },
        }),
    }));
    let Err(withdrawn) = control.sync(&mut Facts::default()).await else {
        panic!("netmap withdrawal")
    };
    // Polling continues: the identity is intact and may get a netmap again.
    assert_eq!(
        withdrawn.recovery(),
        meshway_control::Recovery::Withdrawn { identity: false }
    );
    assert!(control.netmap().is_none());
    control.authenticate().await.unwrap();
    script.push(Ok(full(&binding, &netmap, HOUR)));
    control.sync(&mut Facts::default()).await.unwrap();
    assert!(control.netmap().is_some());
    control.shutdown().await.unwrap();
    server.close().await;
}

#[tokio::test]
async fn stale_signed_same_serial_lease_does_not_replace_the_published_lease() {
    let server = Server::start().await;
    let proxy = Proxy::start(&server.origin).await;
    let (mut control, binding, script) = enrolled(&server, &proxy, "stale-lease").await;
    let netmap = compiled(&binding, "stale-lease", None);
    script.push(Ok(full(&binding, &netmap, HOUR)));
    let before = control
        .sync(&mut Facts::default())
        .await
        .unwrap()
        .netmap
        .lease;
    let mut stale = before;
    stale.expires_at_ms -= 1;
    script.push(Ok(signed(&binding, stale, SyncNetmap::Unchanged)));
    assert!(matches!(
        control.sync(&mut Facts::default()).await,
        Err(Error::Contract)
    ));
    assert_eq!(control.netmap().unwrap().lease, before);
    // A Server clock stepped back: earlier issuedAt, no earlier expiry.
    let mut stepped = before;
    stepped.issued_at_ms -= 5_000;
    script.push(Ok(signed(&binding, stepped, SyncNetmap::Unchanged)));
    control.sync(&mut Facts::default()).await.unwrap();
    assert_eq!(control.netmap().unwrap().lease, stepped);
    control.shutdown().await.unwrap();
    server.close().await;
}
