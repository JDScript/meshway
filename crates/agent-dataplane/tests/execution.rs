use meshway_agent_dataplane::PacketPlan;
use meshway_dataplane::packet_io::Admission;
use meshway_protocol::NodeMeshExecution;

fn execution() -> NodeMeshExecution {
    let a = "a".repeat(64);
    let b = "b".repeat(64);
    serde_json::from_value(serde_json::json!({
        "schema_version":2,"network_id":"0".repeat(32),"revision":7,"node_id":a,
        "required_capability":"ipv4_stateful_exact_path_v2",
        "network":{"ipv4_prefix":"100.64.0.0/24","ipv6_prefix":"fd42::/64"},
        "node":{"id":a,"name":"a","country":null,"mesh_ipv4":"100.64.0.1","mesh_ipv6":"fd42::1"},
        "peers":[{"id":b,"name":"b","country":null,"mesh_ipv4":"100.64.0.2","mesh_ipv6":"fd42::2"}],
        "valid":true,"paths":[{"nodes":[a,b],"return_traffic":"stateful_ipv4_v1"}],"identities":[],"materials":[],
        "connections":{"schema_version":1,"revision":7,"node_id":a,"public_key":null,"capabilities":[],
            "valid":true,"diagnostics":[],"listeners":[],"incoming":[],"outgoing":[],"peers":[]}
    })).unwrap()
}

fn packet(source: [u8; 4], destination: [u8; 4]) -> Vec<u8> {
    let mut bytes = vec![0; 28];
    bytes[0] = 0x45;
    bytes[2..4].copy_from_slice(&28u16.to_be_bytes());
    bytes[8] = 64;
    bytes[9] = 1;
    bytes[12..16].copy_from_slice(&source);
    bytes[16..20].copy_from_slice(&destination);
    bytes[20] = 8;
    bytes[22..24].copy_from_slice(&0xf7ff_u16.to_be_bytes());
    let mut sum: u32 = bytes[..20]
        .as_chunks::<2>()
        .0
        .iter()
        .map(|w| u32::from(u16::from_be_bytes(*w)))
        .sum();
    while sum >> 16 != 0 {
        sum = (sum & 0xffff) + (sum >> 16);
    }
    bytes[10..12].copy_from_slice(&(!(sum as u16)).to_be_bytes());
    bytes
}

#[test]
fn exact_execution_does_not_infer_reverse_authorization_and_withdrawal_retains_only_sink() {
    let mut execution = execution();
    let plan = PacketPlan::compile(&execution, 1000).unwrap();
    let forward = packet([100, 64, 0, 1], [100, 64, 0, 2]);
    let reverse = packet([100, 64, 0, 2], [100, 64, 0, 1]);
    assert!(plan.outgoing(&forward).is_some());
    assert!(plan.outgoing(&reverse).is_none());
    assert!(PacketPlan::compile(&execution, 1400).is_err());
    execution.valid = false;
    assert!(PacketPlan::compile(&execution, 1000).is_err());
    let sink = PacketPlan::sink(&execution, 1000).unwrap();
    assert!(sink.route_policy().retain_fail_closed_sink);
    assert!(sink.outgoing(&forward).is_none());
    assert!(sink.paths.is_empty());
}
