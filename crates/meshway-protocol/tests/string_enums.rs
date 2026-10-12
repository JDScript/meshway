use serde::{Serialize, de::DeserializeOwned};
use serde_json::json;
use utoipa::ToSchema;

fn string_contract<T: DeserializeOwned + Serialize + ToSchema>() {
    let schema = serde_json::to_value(T::schema()).unwrap();
    assert_eq!(schema["type"], "string", "{}", T::name());
    for wire in schema["enum"].as_array().unwrap() {
        let value: T = serde_json::from_value(wire.clone()).unwrap();
        assert_eq!(serde_json::to_value(value).unwrap(), *wire);
        assert!(
            serde_json::from_value::<T>(json!({(wire.as_str().unwrap()):null})).is_err(),
            "{}: {wire}",
            T::name()
        );
    }
    for invalid in [
        json!(null),
        json!(1),
        json!(true),
        json!([]),
        json!({}),
        json!("unknown-enum-value"),
    ] {
        assert!(
            serde_json::from_value::<T>(invalid).is_err(),
            "{}",
            T::name()
        );
    }
}

#[test]
fn shared_wire_unit_enums_accept_exactly_their_published_strings() {
    use meshway_configuration as configuration;
    use meshway_model as model;
    use meshway_protocol as protocol;
    string_contract::<model::MembershipRole>();
    string_contract::<model::KeyPurpose>();
    string_contract::<model::KeyAlgorithm>();
    string_contract::<model::observations::CloseReason>();
    string_contract::<model::observations::MetricKind>();
    string_contract::<model::state::TransportRole>();
    string_contract::<model::state::StreamKind>();
    string_contract::<model::state::ApplyPhase>();
    string_contract::<model::state::SocketNetwork>();
    string_contract::<model::state::ListenerPhase>();
    string_contract::<model::state::LinkPhase>();
    string_contract::<model::state::ReporterRole>();
    string_contract::<model::state::PathEvidence>();
    string_contract::<model::state::ProbeKind>();
    string_contract::<model::state::ProbeOutcome>();
    string_contract::<model::resources::UnavailableReason>();
    string_contract::<model::resources::NotApplicableReason>();
    string_contract::<model::resources::LocationAbsentReason>();
    string_contract::<model::resources::LocationSource>();
    string_contract::<model::resources::AddressScope>();
    string_contract::<protocol::state::AdjacencyConnection>();
    string_contract::<configuration::DnsTransport>();
    string_contract::<configuration::RuleSetFormat>();
    string_contract::<configuration::AddressFamily>();
    string_contract::<configuration::ChangeKind>();
    string_contract::<configuration::AccessDefault>();
    string_contract::<configuration::TopologyDefault>();
    string_contract::<configuration::TopologyAction>();
    string_contract::<configuration::IssueCode>();
    string_contract::<configuration::execution::BusinessProtocol>();
    string_contract::<configuration::execution::LocalEgressAction>();
    string_contract::<configuration::execution::UnavailableReason>();
    string_contract::<configuration::execution::DnsEgressUnavailable>();
    string_contract::<protocol::PasswordLengthUnit>();
    string_contract::<protocol::NetworkNameTrim>();
    string_contract::<protocol::node::ProofOperation>();
    string_contract::<protocol::connectivity::UdpTraversalUnavailableReason>();
    string_contract::<protocol::connectivity::UdpTraversalPhase>();
    string_contract::<protocol::enrollment::SecretDelivery>();
    string_contract::<protocol::enrollment::EnrollmentReturnMethod>();
    string_contract::<protocol::enrollment::EnrollmentPhase>();
    string_contract::<protocol::execution::ReceiptPhase>();
    string_contract::<protocol::execution::ApplyFailure>();
    string_contract::<protocol::execution::ReceiptFreshness>();
    string_contract::<protocol::state::Freshness>();
    string_contract::<protocol::state::CandidateSource>();
    string_contract::<protocol::state::LinkEvidence>();
    string_contract::<protocol::history::ActivityAction>();
    string_contract::<protocol::history::ConfigurationChangeCause>();
    string_contract::<protocol::observations::HistoryMode>();
    string_contract::<protocol::observations::HistoryKind>();
    string_contract::<protocol::observations::CoverageReason>();
    string_contract::<protocol::observations::ObservationPhase>();
    string_contract::<protocol::observations::ReportedObservationPhase>();
    string_contract::<protocol::resources::ResourceProblem>();
    string_contract::<protocol::events::ResetReason>();
    string_contract::<protocol::events::ClosedReason>();
    string_contract::<protocol::live::ProgressScope>();
    string_contract::<protocol::live::ApplicationPhase>();
}

#[test]
fn nested_challenge_and_tagged_state_payloads_do_not_reintroduce_enum_aliases() {
    use meshway_protocol::{NodeChallengeRequest, connectivity::CancelUdpTraversalResponse};
    let id = "00000000-0000-4000-8000-000000000001";
    let mut challenge = json!({"networkId":id,"nodeId":id,"keyId":id,"operation":"session","requestDigest":"00".repeat(32)});
    assert!(serde_json::from_value::<NodeChallengeRequest>(challenge.clone()).is_ok());
    challenge["operation"] = json!({"session":null});
    assert!(serde_json::from_value::<NodeChallengeRequest>(challenge).is_err());
    assert!(
        serde_json::from_value::<CancelUdpTraversalResponse>(
            json!({"status":"unavailable","reason":{"scopeChanged":null}})
        )
        .is_err()
    );
}
