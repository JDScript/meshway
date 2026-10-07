//! Recovery scheduling never grants authority: Control still validates every
//! fresh proof, instance, configuration and receipt before exposing them.
use meshway_control::Error;
use meshway_control_client::ClientError;
use meshway_ipc::Problem;
use std::time::Duration;

pub(super) struct Failure {
    pub problem: Problem,
    pub retry: bool,
    /// Discard the old runtime context before trying a new held-key session.
    pub reset: bool,
    pub retry_after: Option<Duration>,
}

pub(super) fn failure(error: Error, authenticating: bool) -> Failure {
    at("open_control", error, authenticating)
}

pub(super) fn at(operation: &'static str, error: Error, authenticating: bool) -> Failure {
    let (kind, status, code) = details(&error);
    let server_request_id = match &error {
        Error::Server(ClientError::Http {
            problem: Some(body),
            ..
        }) => body.request_id,
        _ => None,
    };
    let retry_after = matches!(&error, Error::StateScopeChanged).then_some(Duration::from_secs(1));
    // A received rejection remains a rejection even if its body is truncated.
    // Runtime bearer/instance rejection first reauthenticates the held identity;
    // rejection of that proof must stop, including unknown Controller/key/Node.
    let authentication_conflict = operation == "authenticate"
        && matches!(&error,
        Error::Server(ClientError::Http { status: 409, problem: Some(body) })
        if body.status == 409 && matches!(body.problem, meshway_protocol::Problem::RequestConflict { .. }));
    let (problem, retry, reset) = match &error {
        Error::MembershipRevoked => (Problem::Credentials, true, true),
        Error::StateScopeChanged => (Problem::Apply, true, false),
        Error::Server(_) if matches!(status, Some(401 | 403 | 404 | 409)) => {
            let retry = !authenticating || authentication_conflict;
            (Problem::Credentials, retry, retry)
        }
        Error::Server(_) if matches!(status, Some(408 | 429 | 500..=599)) => {
            (Problem::ServerBusy, true, false)
        }
        Error::Server(ClientError::Network { .. }) => (Problem::Network, true, false),
        // An HTML/interrupted/oversize response is never a decoded grant. Fence
        // old runtime state and retry delivery, retaining the bound identity.
        Error::Server(ClientError::Decode { .. } | ClientError::BodyLimit { .. }) => {
            (Problem::Network, true, true)
        }
        Error::Expired => (Problem::Clock, true, true),
        Error::Lifecycle if !authenticating => (Problem::Apply, true, true),
        Error::Storage(_) | Error::Record | Error::ReloadRequired => {
            (Problem::Storage, false, false)
        }
        Error::Endpoint => (Problem::Configuration, false, false),
        // A successfully decoded but wrong Controller/key/target/digest remains
        // fatal. No schema or authorization check is relaxed for recovery.
        _ => (Problem::Contract, false, false),
    };
    tracing::warn!(
        operation,
        authenticating,
        error_kind = kind,
        http_status = status,
        ?server_request_id,
        problem_code = code,
        ?problem,
        retry,
        reset,
        "Node control operation failed"
    );
    Failure {
        problem,
        retry,
        reset,
        retry_after,
    }
}

fn details(error: &Error) -> (&'static str, Option<u16>, Option<&'static str>) {
    match error {
        Error::Server(error) => match error {
            ClientError::Network { status } => ("network", *status, None),
            ClientError::Decode { status } => ("decode", Some(*status), None),
            ClientError::BodyLimit { status } => ("body_limit", Some(*status), None),
            ClientError::Http { status, problem } => (
                "http",
                Some(*status),
                problem.as_ref().map(|body| match &body.problem {
                    meshway_protocol::Problem::InvalidCredentials { .. } => "invalid_credentials",
                    meshway_protocol::Problem::PermissionDenied { .. } => "permission_denied",
                    meshway_protocol::Problem::NotFound { .. } => "not_found",
                    meshway_protocol::Problem::RequestConflict { .. } => "request_id_conflict",
                    meshway_protocol::Problem::StateScopeChanged { .. } => "state_scope_changed",
                    meshway_protocol::Problem::Busy { .. } => "busy",
                    meshway_protocol::Problem::ServiceUnavailable { .. } => "service_unavailable",
                    _ => "other",
                }),
            ),
            ClientError::Configuration => ("client_configuration", None, None),
            ClientError::Request => ("client_request", None, None),
        },
        Error::Expired => ("expired", None, None),
        Error::MembershipRevoked => ("membership_revoked", None, None),
        Error::Lifecycle => ("lifecycle", None, None),
        Error::Contract => ("contract", None, None),
        Error::StateScopeChanged => ("state_scope_changed", None, None),
        Error::Storage(_) | Error::Record | Error::ReloadRequired => ("storage", None, None),
        _ => ("local", None, None),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stream_revocation_uses_runtime_credential_recovery_not_operator_blocking() {
        let rejected = at("watch_node", Error::MembershipRevoked, false);
        assert_eq!(rejected.problem, Problem::Credentials);
        assert!(rejected.retry && rejected.reset);
        assert!(!at("authenticate", Error::Contract, true).retry);
    }

    #[test]
    fn only_a_validated_authentication_intent_conflict_can_retry_proof() {
        for body_status in [401, 409] {
            let body = meshway_protocol::ErrorResponse {
                request_id: None,
                status: body_status,
                problem: meshway_protocol::Problem::RequestConflict {
                    params: serde_json::from_value(serde_json::json!({})).unwrap(),
                    message: "not used for classification".into(),
                },
            };
            let result = at(
                "authenticate",
                Error::Server(ClientError::Http {
                    status: 409,
                    problem: Some(body),
                }),
                true,
            );
            assert_eq!(result.retry, body_status == 409);
            assert_eq!(result.reset, body_status == 409);
        }
    }

    #[test]
    fn transient_delivery_and_runtime_expiry_do_not_require_operator_up() {
        for status in [
            408, 429, 500, 502, 503, 504, 520, 521, 522, 523, 524, 525, 526, 530, 599,
        ] {
            for authenticating in [false, true] {
                assert!(
                    failure(
                        Error::Server(ClientError::Http {
                            status,
                            problem: None
                        }),
                        authenticating
                    )
                    .retry
                );
            }
        }
        for error in [
            Error::Expired,
            Error::Lifecycle,
            Error::Server(ClientError::Decode { status: 200 }),
            Error::Server(ClientError::BodyLimit { status: 200 }),
        ] {
            let failed = failure(error, false);
            assert!(failed.retry && failed.reset);
        }
    }

    #[test]
    fn proof_rejections_and_semantic_contract_failures_stay_fatal() {
        for status in [401, 403, 404, 409] {
            for error in [
                ClientError::Http {
                    status,
                    problem: None,
                },
                ClientError::Network {
                    status: Some(status),
                },
                ClientError::Decode { status },
                ClientError::BodyLimit { status },
            ] {
                assert!(!failure(Error::Server(error), true).retry);
            }
            let failed = failure(
                Error::Server(ClientError::Http {
                    status,
                    problem: None,
                }),
                false,
            );
            assert!(failed.retry && failed.reset);
        }
        for error in [
            Error::Contract,
            Error::Record,
            Error::ReloadRequired,
            Error::Lifecycle,
            Error::Cryptography,
        ] {
            assert!(!failure(error, true).retry);
        }
        let failed = failure(Error::StateScopeChanged, false);
        assert!(failed.retry && !failed.reset);
        assert_eq!(failed.retry_after, Some(Duration::from_secs(1)));
    }
}
