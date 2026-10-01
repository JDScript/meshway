use meshway_agent::generated::{self, ApiOpError, HttpClient};
use meshway_protocol::{DEVICE_CALLBACK_URI, local, s256};
use url::Url;

/// No source errors, response bodies or secret values cross this boundary.
#[derive(Debug, thiserror::Error, uniffi::Error, PartialEq, Eq)]
pub enum SignInError {
    #[error("invalid input")]
    InvalidInput,
    #[error("invalid, expired code or mismatched binding")]
    InvalidCode,
    #[error("node credential rejected")]
    CredentialRejected,
    #[error("node revoked")]
    Revoked,
    #[error("credential already bound")]
    CredentialConflict,
    #[error("network capacity exhausted")]
    Capacity,
    #[error("transport unavailable")]
    Transport,
    #[error("unexpected server response")]
    Server,
}

// Intentionally no Debug: this record contains pending secrets.
#[derive(uniffi::Record)]
pub struct AuthorizationAttempt {
    pub url: String,
    pub verifier: String,
    pub state: String,
}

#[derive(uniffi::Record)]
pub struct DeviceNode {
    pub id: String,
    pub name: String,
    pub mesh_ipv4: String,
    pub mesh_ipv6: String,
}

fn origin(value: &str) -> Result<Url, SignInError> {
    let url = Url::parse(value).map_err(|_| SignInError::InvalidInput)?;
    if !matches!(url.scheme(), "http" | "https")
        || url.host_str().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
        || url.path() != "/"
        || (url.scheme() == "http"
            && !matches!(url.host_str(), Some("localhost" | "127.0.0.1" | "[::1]")))
    {
        return Err(SignInError::InvalidInput);
    }
    Ok(url)
}

#[uniffi::export]
pub fn new_node_credential() -> Result<String, SignInError> {
    local::credential().map_err(|_| SignInError::Transport)
}

#[uniffi::export]
pub fn begin_authorization(
    server_url: String,
    name: String,
    app_version: String,
) -> Result<AuthorizationAttempt, SignInError> {
    let mut url = origin(&server_url)?;
    let verifier = new_node_credential()?;
    let state = new_node_credential()?;
    let challenge = s256(&verifier);
    meshway_protocol::AuthorizeDeviceRequest {
        redirect_uri: DEVICE_CALLBACK_URI.into(),
        code_challenge: challenge.clone(),
        name: name.clone(),
        agent_version: app_version.clone(),
    }
    .validate()
    .map_err(|_| SignInError::InvalidInput)?;
    url.set_path("/authorize-device");
    url.query_pairs_mut().extend_pairs([
        ("redirect_uri", DEVICE_CALLBACK_URI),
        ("state", &state),
        ("code_challenge", &challenge),
        ("name", &name),
        ("agent_version", &app_version),
    ]);
    Ok(AuthorizationAttempt {
        url: url.into(),
        verifier,
        state,
    })
}

pub(crate) fn client(server: &str, bearer: &str) -> Result<HttpClient, SignInError> {
    let base = origin(server)?;
    if !local::valid_credential(bearer) {
        return Err(SignInError::InvalidInput);
    }
    Ok(HttpClient::new()
        .with_base_url(base.as_str().trim_end_matches('/'))
        .with_api_key(bearer)
        .with_max_response_body_bytes(64 * 1024))
}

pub(crate) fn sanitized<E: std::fmt::Debug>(error: ApiOpError<E>) -> SignInError {
    match error {
        ApiOpError::Transport(_) => SignInError::Transport,
        ApiOpError::Api(error) => match error.status {
            400 | 413 | 415 => SignInError::InvalidInput,
            401 => SignInError::InvalidCode,
            403 => SignInError::Revoked,
            409 => SignInError::CredentialConflict,
            422 => SignInError::Capacity,
            _ => SignInError::Server,
        },
    }
}

fn node(response: generated::AgentNodeResponse) -> Result<DeviceNode, SignInError> {
    if response.node.revoked_at.is_some() {
        return Err(SignInError::Revoked);
    }
    Ok(DeviceNode {
        id: response.node.id,
        name: response.node.name,
        mesh_ipv4: response.node.mesh_ipv4,
        mesh_ipv6: response.node.mesh_ipv6,
    })
}

// A single bounded bridge-owned runtime keeps reqwest off Swift's executor.
// The exported futures only await a task handle; no tokio context is required
// on the thread polling UniFFI. No extra async compatibility dependency.
pub(crate) async fn on_runtime<T: Send + 'static>(
    future: impl std::future::Future<Output = Result<T, SignInError>> + Send + 'static,
) -> Result<T, SignInError> {
    static RUNTIME: std::sync::OnceLock<Result<tokio::runtime::Runtime, std::io::Error>> =
        std::sync::OnceLock::new();
    let runtime = RUNTIME
        .get_or_init(|| {
            tokio::runtime::Builder::new_multi_thread()
                .worker_threads(1)
                .enable_all()
                .build()
        })
        .as_ref()
        .map_err(|_| SignInError::Transport)?;
    runtime
        .spawn(future)
        .await
        .map_err(|_| SignInError::Transport)?
}

#[uniffi::export]
pub async fn exchange_device(
    server_url: String,
    code: String,
    verifier: String,
    credential: String,
    name: String,
    app_version: String,
) -> Result<DeviceNode, SignInError> {
    let validated = meshway_protocol::ExchangeDeviceRequest {
        code_verifier: verifier,
        redirect_uri: DEVICE_CALLBACK_URI.into(),
        node_credential: credential,
        name,
        agent_version: app_version,
    };
    validated
        .enrollment()
        .map_err(|_| SignInError::InvalidInput)?;
    on_runtime(async move {
        let response = client(&server_url, &code)?
            .exchange_device(generated::ExchangeDeviceRequest {
                code_verifier: validated.code_verifier,
                redirect_uri: validated.redirect_uri,
                node_credential: validated.node_credential,
                name: validated.name,
                agent_version: validated.agent_version,
            })
            .await
            .map_err(sanitized)?;
        node(response)
    })
    .await
}

#[uniffi::export]
pub async fn heartbeat(server_url: String, credential: String) -> Result<DeviceNode, SignInError> {
    let request = meshway_protocol::HeartbeatRequest {
        agent_version: env!("CARGO_PKG_VERSION").into(),
    };
    request.validate().map_err(|_| SignInError::InvalidInput)?;
    on_runtime(async move {
        node(
            client(&server_url, &credential)?
                .heartbeat_agent(generated::HeartbeatRequest {
                    agent_version: request.agent_version,
                })
                .await
                .map_err(|error| {
                    if error.api().is_some_and(|error| error.status == 401) {
                        SignInError::CredentialRejected
                    } else {
                        sanitized(error)
                    }
                })?,
        )
    })
    .await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn authorization_shape_encoding_and_shared_s256() {
        let attempt = begin_authorization(
            "https://console.test".into(),
            "Mac & 中文".into(),
            "v+1".into(),
        )
        .unwrap();
        let url = Url::parse(&attempt.url).unwrap();
        assert_eq!(url.path(), "/authorize-device");
        let query: std::collections::HashMap<_, _> = url.query_pairs().collect();
        assert_eq!(query["redirect_uri"], DEVICE_CALLBACK_URI);
        assert_eq!(query["code_challenge"], s256(&attempt.verifier));
        assert_eq!(query["state"], attempt.state);
        assert_eq!(query["name"], "Mac & 中文");
        assert_eq!(query["agent_version"], "v+1");
        assert!(!attempt.url.contains(&attempt.verifier));
        assert_eq!(attempt.verifier.len(), 64);
        let fresh =
            begin_authorization("https://console.test".into(), "Mac".into(), "v1".into()).unwrap();
        assert_ne!(fresh.state, attempt.state);
        assert_ne!(fresh.verifier, attempt.verifier);
        for bad in [
            "https://user:secret@test",
            "https://test/path",
            "http://remote.test",
            "https://test?secret=x",
        ] {
            assert!(matches!(
                begin_authorization(bad.into(), "Mac".into(), "v1".into()),
                Err(SignInError::InvalidInput)
            ));
        }
        assert_eq!(
            s256("dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk"),
            "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM"
        );
    }
}
