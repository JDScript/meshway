#![cfg(unix)]

use std::time::Duration;

use meshway_controller::{
    App, AppError, Settings,
    accounts::{AccountError, Secret},
};
use meshway_model::RequestId;
use uuid::Uuid;

fn request() -> RequestId {
    let mut bytes = [0; 16];
    getrandom::fill(&mut bytes).unwrap();
    RequestId::new(Uuid::from_bytes(bytes)).unwrap()
}

fn secret(value: &str) -> Secret {
    Secret::new(value.into())
}

#[tokio::test]
async fn public_account_lifecycle_and_input_contract() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().canonicalize().unwrap().join("accounts");
    let mut settings = Settings::new(&path);
    settings.session_ttl = Duration::from_secs(90);
    let app = App::open(settings).await.unwrap();
    let accounts = app.accounts();
    let options = accounts.options();
    assert!(options.registration_enabled);
    assert_eq!(options.password.min_codepoints, 12);
    assert_eq!(options.password.max_codepoints, 128);
    assert_eq!(options.password.max_utf8_bytes, 512);
    // Spaces are password content, not normalization input.
    let password = " 0123456789 ";
    let alice = accounts
        .register(" Alice@BÜCHER.example ".into(), secret(password), request())
        .await
        .unwrap();
    assert_eq!(alice.session.user.email, "alice@xn--bcher-kva.example");
    assert_eq!(
        (alice.session.expires_at - alice.session.created_at).num_seconds(),
        90
    );
    assert_eq!(alice.token.expose().len(), 68);
    assert!(alice.token.expose().starts_with("mws_"));
    assert_eq!(
        accounts.authenticate(alice.token.expose()).await.unwrap(),
        alice.session
    );
    let duplicate = accounts
        .register(
            "ALICE@xn--bcher-kva.example".into(),
            secret("different password"),
            request(),
        )
        .await;
    assert!(matches!(duplicate, Err(AccountError::EmailTaken)));
    for email in [
        "missing@example.test",
        "bad@@example.test",
        "alice@xn--bcher-kva.example",
    ] {
        assert!(matches!(
            accounts
                .login(email.into(), secret("wrong password"), request())
                .await,
            Err(AccountError::InvalidCredentials)
        ));
    }
    // Moving spaces changes the password even at the same valid length.
    assert!(matches!(
        accounts
            .login(
                "alice@xn--bcher-kva.example".into(),
                secret("0123456789  "),
                request()
            )
            .await,
        Err(AccountError::InvalidCredentials)
    ));
    let again = accounts
        .login("alice@bücher.example".into(), secret(password), request())
        .await
        .unwrap();
    assert_eq!(again.session.user.id, alice.session.user.id);
    assert_ne!(again.session.session_id, alice.session.session_id);
    assert_ne!(again.token.expose(), alice.token.expose());
    let bob_password = "🦀".repeat(128);
    let bob = accounts
        .register(
            "bob@example.test".into(),
            Secret::new(bob_password.clone()),
            request(),
        )
        .await
        .unwrap();
    assert_ne!(bob.session.user.id, alice.session.user.id);
    for value in ["🦀".repeat(11), "a".repeat(129), "🦀".repeat(129)] {
        assert!(matches!(
            accounts
                .register("invalid@example.test".into(), Secret::new(value), request())
                .await,
            Err(AccountError::InvalidPassword)
        ));
    }
    for email in [
        "a..b@example.test",
        "\nuser@example.test",
        "用户@example.test",
        "a@-host.test",
        "a@host.test.",
        "\"a\"@example.test",
    ] {
        assert!(matches!(
            accounts
                .register(email.into(), secret(password), request())
                .await,
            Err(AccountError::InvalidEmail)
        ));
    }
    accounts
        .logout(alice.token.expose(), request())
        .await
        .unwrap();
    assert!(matches!(
        accounts.authenticate(alice.token.expose()).await,
        Err(AccountError::InvalidSession)
    ));
    assert!(matches!(
        accounts.logout(alice.token.expose(), request()).await,
        Err(AccountError::InvalidSession)
    ));
    accounts.authenticate(again.token.expose()).await.unwrap();
    accounts.authenticate(bob.token.expose()).await.unwrap();
    app.check_storage().await.unwrap();
    app.shutdown().await.unwrap();
    assert!(matches!(
        accounts.authenticate(bob.token.expose()).await,
        Err(AccountError::Unavailable)
    ));

    // Inspect actual persisted bytes after SQLite closes (including any files
    // left by SQLite), not a mock serialization of a persistence DTO.
    for entry in std::fs::read_dir(&path).unwrap() {
        let bytes = std::fs::read(entry.unwrap().path()).unwrap();
        for raw in [
            password,
            &bob_password,
            alice.token.expose(),
            bob.token.expose(),
            again.token.expose(),
        ] {
            assert!(
                !bytes
                    .windows(raw.len())
                    .any(|window| window == raw.as_bytes())
            );
        }
    }
    let mut settings = Settings::new(&path);
    settings.registration_enabled = false;
    let app = App::open(settings).await.unwrap();
    let accounts = app.accounts();
    assert!(!accounts.options().registration_enabled);
    assert!(matches!(
        accounts
            .register("new@example.test".into(), secret(password), request())
            .await,
        Err(AccountError::RegistrationDisabled)
    ));
    assert_eq!(
        accounts.authenticate(again.token.expose()).await.unwrap(),
        again.session
    );
    assert!(matches!(
        accounts.authenticate(alice.token.expose()).await,
        Err(AccountError::InvalidSession)
    ));
    accounts
        .login("alice@bücher.example".into(), secret(password), request())
        .await
        .unwrap();
    app.shutdown().await.unwrap();
}

#[tokio::test]
async fn ttl_limits_are_validated_before_filesystem_startup() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().canonicalize().unwrap().join("accounts");
    for ttl in [
        Duration::from_secs(59),
        Duration::from_secs(30 * 86400 + 1),
        Duration::from_millis(60500),
    ] {
        let mut settings = Settings::new(&path);
        settings.session_ttl = ttl;
        assert!(matches!(
            App::open(settings).await,
            Err(AppError::InvalidSettings)
        ));
        assert!(!path.exists());
    }
}
