mod support;

use meshway_control_client::ClientError;
use meshway_protocol::{
    ErrorResponse, LoginRequest, PasswordLengthUnit, Problem, RegisterRequest, WireSecret,
};

fn password() -> WireSecret {
    WireSecret::new("🔐".repeat(16))
}

#[tokio::test]
async fn generated_sdk_accounts_use_normal_controller_storage_and_sessions() {
    let (app, client, server, _directory) = support::controller(true).await;
    let options = client.get_auth_options().await.unwrap();
    assert!(options.registration_enabled);
    assert_eq!(
        options.password_rules.length_unit,
        PasswordLengthUnit::UnicodeCodepoints
    );
    assert!(options.password_rules.min_codepoints <= 16);
    assert!(options.password_rules.max_utf8_bytes >= 64);

    let issued = client
        .register(&RegisterRequest {
            email: "  ALICE@BÜCHER.example  ".into(),
            password: password(),
        })
        .await
        .unwrap();
    assert_eq!(issued.user.email, "alice@xn--bcher-kva.example");
    assert!(issued.session.expires_at > issued.session.created_at);
    let session = client
        .get_auth_session(issued.token.expose())
        .await
        .unwrap();
    assert_eq!(session.user.id, issued.user.id);
    assert_eq!(session.session.id, issued.session.id);

    let duplicate = client
        .register(&RegisterRequest {
            email: issued.user.email.clone(),
            password: password(),
        })
        .await;
    assert!(matches!(
        duplicate,
        Err(ClientError::Http {
            status: 409,
            problem: Some(ErrorResponse {
                problem: Problem::EmailTaken { .. },
                ..
            })
        })
    ));
    let weak = client
        .register(&RegisterRequest {
            email: "weak@example.com".into(),
            password: WireSecret::new("short".into()),
        })
        .await;
    match weak {
        Err(ClientError::Http {
            status: 422,
            problem:
                Some(ErrorResponse {
                    problem: Problem::PasswordWeak { params, .. },
                    ..
                }),
        }) => assert_eq!(params, options.password_rules),
        _ => panic!("expected typed password rules"),
    }
    let wrong = client
        .login(&LoginRequest {
            email: issued.user.email.clone(),
            password: WireSecret::new("wrong password".into()),
        })
        .await;
    assert!(matches!(
        wrong,
        Err(ClientError::Http {
            status: 401,
            problem: Some(ErrorResponse {
                problem: Problem::InvalidCredentials { .. },
                ..
            })
        })
    ));
    client.logout(issued.token.expose()).await.unwrap();
    assert!(matches!(
        client.get_auth_session(issued.token.expose()).await,
        Err(ClientError::Http {
            status: 401,
            problem: Some(ErrorResponse {
                problem: Problem::InvalidSession { .. },
                ..
            })
        })
    ));

    let logged_in = client
        .login(&LoginRequest {
            email: issued.user.email,
            password: password(),
        })
        .await
        .unwrap();
    assert_eq!(logged_in.user.id, issued.user.id);
    assert_ne!(logged_in.session.id, issued.session.id);
    client.logout(logged_in.token.expose()).await.unwrap();
    app.check_storage().await.unwrap();
    server.0.abort();
    app.shutdown().await.unwrap();
}

#[tokio::test]
async fn registration_switch_is_enforced_by_the_controller() {
    let (app, client, server, _directory) = support::controller(false).await;
    assert!(
        !client
            .get_auth_options()
            .await
            .unwrap()
            .registration_enabled
    );
    assert!(matches!(
        client
            .register(&RegisterRequest {
                email: "disabled@example.com".into(),
                password: password()
            })
            .await,
        Err(ClientError::Http {
            status: 403,
            problem: Some(ErrorResponse {
                problem: Problem::RegistrationDisabled { .. },
                ..
            })
        })
    ));
    server.0.abort();
    app.shutdown().await.unwrap();
}
