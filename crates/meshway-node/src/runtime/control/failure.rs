//! Recovery scheduling never grants authority: Control still validates every
//! fresh proof, instance, configuration and receipt before exposing them.
use meshway_control::Error;
use meshway_control_client::ClientError;
use meshway_protocol::ipc::Problem;
use std::time::Duration;

pub(super) struct Failure {
    pub problem: Problem,
    /// Why, for a local storage failure (no remote text or secret): Status
    /// `lastErrorDetail`.
    pub detail: Option<String>,
    pub retry: bool,
    /// The old runtime context (session, instance) is gone and is re-acquired
    /// at once; the data plane follows netmap authority regardless.
    pub reset: bool,
    /// The held netmap ended (decoded withdrawal) or local storage is in an
    /// unknown state: the data plane is withdrawn. Every other failure keeps
    /// it running on its signed lease.
    pub withdraw: bool,
    pub retry_after: Option<Duration>,
}

/// A failed `Control::open`, judged by [`Error::open_recovery`]: a held
/// lock or a failed read is retried, a proven inconsistency blocks.
pub(super) fn failure(error: Error, authenticating: bool) -> Failure {
    let recovery = error.open_recovery();
    judged("open_control", error, recovery, authenticating)
}

/// Control's [`Recovery`](meshway_control::Recovery) decides retry and
/// withdrawal; this adds only the reported problem and pacing.
pub(super) fn at(operation: &'static str, error: Error, authenticating: bool) -> Failure {
    let recovery = error.recovery();
    judged(operation, error, recovery, authenticating)
}

fn judged(
    operation: &'static str,
    error: Error,
    recovery: meshway_control::Recovery,
    authenticating: bool,
) -> Failure {
    use meshway_control::Recovery;
    let (kind, status, code) = details(&error);
    let server_request_id = match &error {
        Error::ReceiptRejected(rejection) => rejection.response().request_id,
        Error::Server(ClientError::Http {
            problem: Some(body),
            ..
        }) => body.request_id,
        _ => None,
    };
    let retry_after = matches!(&error, Error::ReceiptRejected(_)).then_some(Duration::from_secs(1));
    let (retry, reset, withdraw) = match recovery {
        Recovery::Retry { reset } => (true, reset, false),
        Recovery::Withdrawn { identity } => (!identity, true, true),
        Recovery::Storage => (false, false, true),
        Recovery::Halt => (false, true, false),
    };
    let problem = match &error {
        Error::ReceiptRejected(_) => Problem::Apply,
        _ if matches!(recovery, Recovery::Withdrawn { .. }) => Problem::Credentials,
        Error::Server(_) if matches!(status, Some(401 | 403 | 404 | 409)) => Problem::Credentials,
        Error::Server(_) if matches!(status, Some(408 | 429 | 500..=599)) => Problem::ServerBusy,
        Error::Server(
            ClientError::Network { .. }
            | ClientError::Decode { .. }
            | ClientError::BodyLimit { .. },
        ) => Problem::Network,
        Error::Server(ClientError::Http { .. }) => Problem::ServerBusy,
        Error::Expired => Problem::Clock,
        Error::Lifecycle | Error::Pending | Error::StateScopeChanged | Error::StaleReport => {
            Problem::Apply
        }
        Error::Storage(_) | Error::Record(_) | Error::Origin { .. } | Error::ReloadRequired => {
            Problem::Storage
        }
        Error::Endpoint => Problem::Configuration,
        _ => Problem::Contract,
    };
    let detail = (problem == Problem::Storage).then(|| error.to_string());
    tracing::warn!(
        operation,
        authenticating,
        detail = detail.as_deref(),
        error_kind = kind,
        http_status = status,
        ?server_request_id,
        problem_code = code,
        ?problem,
        retry,
        reset,
        withdraw,
        "Node control operation failed"
    );
    Failure {
        problem,
        detail,
        retry,
        reset,
        withdraw,
        retry_after,
    }
}

impl Failure {
    pub fn cause(&self) -> super::Cause {
        super::Cause {
            problem: self.problem,
            detail: self.detail.clone(),
        }
    }
}

fn details(error: &Error) -> (&'static str, Option<u16>, Option<&'static str>) {
    match error {
        Error::ReceiptRejected(rejection) => (
            "receipt_rejected",
            Some(rejection.response().status),
            Some(problem_code(&rejection.response().problem)),
        ),
        Error::Server(error) => match error {
            ClientError::Network { status } => ("network", *status, None),
            ClientError::Decode { status } => ("decode", Some(*status), None),
            ClientError::BodyLimit { status } => ("body_limit", Some(*status), None),
            ClientError::Http { status, problem } => (
                "http",
                Some(*status),
                problem.as_ref().map(|body| problem_code(&body.problem)),
            ),
            ClientError::Configuration => ("client_configuration", None, None),
            ClientError::Request => ("client_request", None, None),
        },
        Error::Expired => ("expired", None, None),
        Error::Lifecycle => ("lifecycle", None, None),
        Error::Contract => ("contract", None, None),
        Error::StateScopeChanged => ("state_scope_changed", None, None),
        Error::StaleReport => ("stale_report", None, None),
        Error::Storage(_) | Error::Record(_) | Error::Origin { .. } | Error::ReloadRequired => {
            ("storage", None, None)
        }
        _ => ("local", None, None),
    }
}

fn problem_code(problem: &meshway_protocol::Problem) -> &'static str {
    match problem {
        meshway_protocol::Problem::InvalidCredentials { .. } => "invalid_credentials",
        meshway_protocol::Problem::PermissionDenied { .. } => "permission_denied",
        meshway_protocol::Problem::NotFound { .. } => "not_found",
        meshway_protocol::Problem::RequestConflict { .. } => "request_id_conflict",
        meshway_protocol::Problem::BindingConflict { .. } => "binding_conflict",
        meshway_protocol::Problem::ReceiptInstanceMismatch { .. } => "receipt_instance_mismatch",
        meshway_protocol::Problem::ReceiptTargetMismatch { .. } => "receipt_target_mismatch",
        meshway_protocol::Problem::ReceiptNetmapSuperseded { .. } => "receipt_netmap_superseded",
        meshway_protocol::Problem::ReceiptPrecondition { .. } => "receipt_precondition",
        meshway_protocol::Problem::ReceiptContentConflict { .. } => "receipt_content_conflict",
        meshway_protocol::Problem::StateScopeChanged { .. } => "state_scope_changed",
        meshway_protocol::Problem::Busy { .. } => "busy",
        meshway_protocol::Problem::ServiceUnavailable { .. } => "service_unavailable",
        _ => "other",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn body(status: u16, code: &str) -> meshway_protocol::ErrorResponse {
        serde_json::from_value(serde_json::json!({
            "requestId": null, "status": status,
            "problem": {"code": code, "params": {}, "message": "ignored"}
        }))
        .unwrap()
    }

    #[test]
    fn receipt_conflicts_only_halt_for_target_binding_rejection() {
        for (code, refresh) in [
            ("receipt_netmap_superseded", true),
            ("receipt_precondition", true),
            ("receipt_content_conflict", true),
            ("receipt_instance_mismatch", false),
            ("receipt_target_mismatch", false),
            ("binding_conflict", false),
            ("request_id_conflict", false),
        ] {
            let body = body(409, code);
            // Control only exposes ReceiptRejected for validated receipt-domain
            // responses; its HTTP integration tests exercise that boundary.
            let error = if refresh {
                Error::ReceiptRejected(Box::new(body.into()))
            } else {
                Error::Server(ClientError::Http {
                    status: 409,
                    problem: Some(body),
                })
            };
            let failure = at("finish_application", error, false);
            assert_eq!(failure.retry, code != "receipt_target_mismatch", "{code}");
            assert_eq!(failure.reset, !refresh, "{code}");
            assert!(!failure.withdraw, "{code}: the lease keeps running");
            assert_eq!(
                failure.retry_after,
                refresh.then_some(Duration::from_secs(1))
            );
        }
    }

    #[test]
    fn only_a_decoded_withdrawal_or_unknown_storage_withdraws_the_data_plane() {
        let rejected = at(
            "authenticate",
            Error::Server(ClientError::Http {
                status: 403,
                problem: Some(body(403, "node_identity_rejected")),
            }),
            true,
        );
        assert!(rejected.withdraw && !rejected.retry);
        assert_eq!(rejected.problem, Problem::Credentials);
        let withdrawn = at(
            "sync",
            Error::Server(ClientError::Http {
                status: 403,
                problem: Some(body(403, "netmap_withdrawn")),
            }),
            false,
        );
        assert!(
            withdrawn.withdraw && withdrawn.retry,
            "keeps polling for a new netmap"
        );
        for error in [Error::Record("test"), Error::ReloadRequired] {
            let failed = failure(error, false);
            assert!(failed.withdraw && !failed.retry);
        }
        for error in [
            Error::Contract,
            Error::Endpoint,
            Error::Cryptography,
            Error::SignatureInvalid,
            Error::ServerKeyMismatch,
        ] {
            let failed = failure(error, true);
            assert!(!failed.retry && !failed.withdraw, "halts control only");
        }
    }

    /// N8: opening never writes the identity record, so a held lock or a
    /// failed read is unknown and retried; a write elsewhere keeps the
    /// RC-D Q6 exception, and a proven inconsistency blocks.
    #[test]
    fn an_open_retries_a_held_lock_or_failed_read_and_blocks_a_proven_one() {
        use meshway_private_storage::Error as Storage;
        for storage in [(|| Storage::InUse) as fn() -> Storage, || Storage::Io] {
            let opened = failure(Error::Storage(storage()), true);
            assert!(opened.retry && !opened.withdraw);
            assert_eq!(opened.problem, Problem::Storage);
            assert!(opened.detail.is_some(), "Status names the cause");
            let written = at("persist", Error::Storage(storage()), true);
            assert!(!written.retry && written.withdraw, "RC-D Q6 exception");
        }
        for error in [
            Error::Storage(Storage::UnsafeStorage),
            Error::Storage(Storage::InvalidPath),
            Error::Record("control.json is not a valid identity record"),
            Error::Origin {
                held: meshway_control::Held::Identity,
                origin: "https://a.example".into(),
                configured: "https://b.example".into(),
            },
        ] {
            let opened = failure(error, true);
            assert!(!opened.retry && opened.detail.is_some());
        }
    }

    #[test]
    fn undecoded_proxy_statuses_and_other_rejections_retry_and_keep_the_lease() {
        for authenticating in [false, true] {
            for status in [400, 401, 403, 404, 409, 410, 422] {
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
                    let failed = failure(Error::Server(error), authenticating);
                    assert!(failed.retry && !failed.withdraw, "{status}");
                }
            }
            // Decoded, but neither a withdrawal nor a contract break: a stale
            // observation's 400 or a clock-skewed proof's 401 are retried.
            for (status, code) in [
                (400, "invalid_request"),
                (401, "invalid_credentials"),
                (403, "permission_denied"),
            ] {
                let failed = failure(
                    Error::Server(ClientError::Http {
                        status,
                        problem: Some(body(status, code)),
                    }),
                    authenticating,
                );
                assert!(failed.retry && !failed.withdraw, "{code}");
            }
        }
    }

    #[test]
    fn transient_delivery_and_runtime_expiry_do_not_require_operator_up() {
        for status in [
            408, 429, 500, 502, 503, 504, 520, 521, 522, 523, 524, 525, 526, 530, 599,
        ] {
            for authenticating in [false, true] {
                let failed = failure(
                    Error::Server(ClientError::Http {
                        status,
                        problem: None,
                    }),
                    authenticating,
                );
                assert!(failed.retry && !failed.withdraw);
                assert_eq!(failed.problem, Problem::ServerBusy);
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
}
