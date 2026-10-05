//! Safe failure metadata. Error messages from HTTP bodies, serde values and
//! transport providers are never formatted into the process log.
use meshway_control_client::ClientError;
use std::error::Error;

#[derive(Debug, PartialEq, Eq)]
struct Metadata<'a> {
    category: &'static str,
    detail: &'static str,
    signal: Option<&'a str>,
    http_status: Option<u16>,
    line: Option<usize>,
    column: Option<usize>,
}
impl Metadata<'_> {
    fn new(category: &'static str, detail: &'static str) -> Self {
        Self {
            category,
            detail,
            signal: None,
            http_status: None,
            line: None,
            column: None,
        }
    }
}
pub(super) fn json_category(error: &serde_json::Error) -> &'static str {
    match error.classify() {
        serde_json::error::Category::Io => "io",
        serde_json::error::Category::Syntax => "syntax",
        serde_json::error::Category::Data => "data",
        serde_json::error::Category::Eof => "eof",
    }
}
fn client<P>(error: &ClientError<P>) -> Metadata<'static> {
    let (detail, status) = match error {
        ClientError::Configuration => ("configuration", None),
        ClientError::Request => ("request", None),
        ClientError::Network { status } => ("network", *status),
        ClientError::BodyLimit { status } => ("body_limit", Some(*status)),
        ClientError::Decode { status } => ("decode", Some(*status)),
        ClientError::Http { status, .. } => ("http", Some(*status)),
    };
    let mut value = Metadata::new("server_client", detail);
    value.http_status = status;
    value
}
fn metadata<'a>(error: &'a (dyn Error + 'static)) -> Metadata<'a> {
    if let Some(error) = error.downcast_ref::<crate::signals::DecodeError>() {
        let mut value = Metadata::new("rendezvous_decode", error.detail());
        value.signal = Some(&error.signal);
        value.line = (error.source.line() > 0).then_some(error.source.line());
        value.column = value.line.map(|_| error.source.column());
        return value;
    }
    if let Some(error) = error.downcast_ref::<serde_json::Error>() {
        let mut value = Metadata::new("json", json_category(error));
        value.line = (error.line() > 0).then_some(error.line());
        value.column = value.line.map(|_| error.column());
        return value;
    }
    if let Some(error) = error.downcast_ref::<ClientError<meshway_protocol::ErrorResponse>>() {
        return client(error);
    }
    if let Some(error) = error.downcast_ref::<ClientError<std::convert::Infallible>>() {
        return client(error);
    }
    if let Some(error) = error.downcast_ref::<std::io::Error>() {
        let detail = match error.kind() {
            std::io::ErrorKind::NotFound => "not_found",
            std::io::ErrorKind::PermissionDenied => "permission_denied",
            std::io::ErrorKind::TimedOut => "timed_out",
            std::io::ErrorKind::ConnectionReset => "connection_reset",
            std::io::ErrorKind::BrokenPipe => "broken_pipe",
            _ => "other",
        };
        return Metadata::new("io", detail);
    }
    if error.is::<tokio::time::error::Elapsed>() {
        return Metadata::new("timeout", "elapsed");
    }
    if let Some(error) = error.downcast_ref::<tokio::task::JoinError>() {
        return Metadata::new(
            "task",
            if error.is_cancelled() {
                "cancelled"
            } else {
                "panicked"
            },
        );
    }
    // Only exact static fixture assertions are safe for the ordinary log.
    let detail = match error.to_string().as_str() {
        "incomplete runtime actor evidence" => "incomplete_actor_report",
        "final recovery report is bound to a different Node" => "report_identity",
        "outage did not expire the unchanged daemon's actual instance" => "instance_expiry",
        "automatic recovery changed operator identity/intent or reused expired authority" => {
            "recovery_authority"
        }
        "recovery lacks old-flow rejection or fresh socket/circuit evidence" => "recovery_flows",
        "terminal did not receive new dual-stack UDP/TCP flows" => "terminal_delivery",
        "recovered relay delivered business plaintext" => "relay_delivery",
        _ => return Metadata::new("unclassified", "inspect_private_evidence"),
    };
    Metadata::new("fixture_assertion", detail)
}
pub(super) fn log(error: &(dyn Error + 'static)) {
    let value = metadata(error);
    tracing::error!(
        failure_category = value.category,
        failure_detail = value.detail,
        signal = value.signal,
        http_status = value.http_status,
        json_line = value.line,
        json_column = value.column,
        "runtime fixture failed; inspect last stage and private evidence"
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn diagnostics_never_include_arbitrary_serde_io_or_http_body_text() {
        let sentinel = "credential-sentinel-never-log";
        let error = serde_json::from_str::<u64>(&format!("\"{sentinel}\"")).unwrap_err();
        assert_eq!(metadata(&error).category, "json");
        assert!(!format!("{:?}", metadata(&error)).contains(sentinel));
        let error = std::io::Error::other(sentinel);
        assert_eq!(metadata(&error).category, "io");
        assert!(!format!("{:?}", metadata(&error)).contains(sentinel));
        let error: ClientError<meshway_protocol::ErrorResponse> = ClientError::Http {
            status: 503,
            problem: Some(
                serde_json::from_value(serde_json::json!({
                    "requestId":null,"status":503,
                    "problem":{"code":"service_unavailable","params":{},"message":sentinel}
                }))
                .unwrap(),
            ),
        };
        assert_eq!(metadata(&error).http_status, Some(503));
        assert!(!format!("{:?}", metadata(&error)).contains(sentinel));
        let error = crate::signals::Signal::decode("report-a", br#"{"kind":"report"}"#)
            .err()
            .unwrap();
        assert_eq!(metadata(&error).category, "rendezvous_decode");
        assert_eq!(metadata(&error).signal, Some("report-a"));
    }
}
