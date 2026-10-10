use std::collections::{BTreeMap, BTreeSet};

use meshway_model::{
    ActivityId, InstanceId, InvalidId, JoinLinkId, KeyId, MembershipId, NetworkId, NodeId,
    RequestId, SessionId, UserId,
};
use uuid::Uuid;

const ID: &str = "01234567-89ab-cdef-0123-456789abcdef";

macro_rules! check_id {
    ($test:ident, $type:ty) => {
        #[test]
        fn $test() {
            let value: $type = ID.parse().unwrap();
            let uuid = Uuid::parse_str(ID).unwrap();
            assert_eq!(<$type>::new(uuid).unwrap(), value);
            assert_eq!(<$type>::try_from(uuid).unwrap(), value);
            assert_eq!(*value.as_uuid(), uuid);
            assert_eq!(value.into_uuid(), uuid);
            assert_eq!(value.to_string(), ID);
            let json = format!("\"{ID}\"");
            assert_eq!(serde_json::to_string(&value).unwrap(), json);
            assert_eq!(serde_json::from_str::<$type>(&json).unwrap(), value);

            assert_eq!(<$type>::new(Uuid::nil()), Err(InvalidId));
            assert_eq!(<$type>::try_from(Uuid::nil()), Err(InvalidId));
            for invalid in [
                "".to_owned(),
                "00000000-0000-0000-0000-000000000000".to_owned(),
                ID.to_uppercase(),
                ID.replace('-', ""),
                format!("{{{ID}}}"),
                format!("urn:uuid:{ID}"),
                format!(" {ID}"),
                format!("{ID}\n"),
                ID.replace('a', "g"),
                ID.replace('-', "_"),
                ID.replace('a', "ａ"),
                format!("{ID}\0"),
                ID[..35].to_owned(),
            ] {
                assert!(invalid.parse::<$type>().is_err(), "{invalid:?}");
                let encoded = serde_json::to_string(&invalid).unwrap();
                assert!(
                    serde_json::from_str::<$type>(&encoded).is_err(),
                    "{invalid:?}"
                );
            }
            for invalid in ["null", "1", "true", "[]", "{}"] {
                assert!(serde_json::from_str::<$type>(invalid).is_err());
            }

            // No UUID version restriction or accidental exclusion of a nonzero boundary.
            for valid in [
                "00000000-0000-0000-0000-000000000001",
                "ffffffff-ffff-ffff-ffff-ffffffffffff",
            ] {
                assert_eq!(valid.parse::<$type>().unwrap().to_string(), valid);
            }
        }
    };
}

check_id!(user_id, UserId);
check_id!(network_id, NetworkId);
check_id!(node_id, NodeId);
check_id!(membership_id, MembershipId);
check_id!(session_id, SessionId);
check_id!(request_id, RequestId);
check_id!(instance_id, InstanceId);
check_id!(key_id, KeyId);
check_id!(activity_id, ActivityId);
check_id!(join_link_id, JoinLinkId);

#[test]
fn identifiers_order_by_uuid_and_round_trip_as_configuration_keys() {
    let first: NodeId = "00000000-0000-0000-0000-000000000001".parse().unwrap();
    let second: NodeId = ID.parse().unwrap();
    let last: NodeId = "ffffffff-ffff-ffff-ffff-ffffffffffff".parse().unwrap();
    let ordered = BTreeSet::from([last, second, first]);
    assert_eq!(
        ordered.into_iter().collect::<Vec<_>>(),
        [first, second, last]
    );

    let map = BTreeMap::from([(first, "first".to_owned()), (second, "second".to_owned())]);
    let encoded = serde_json::to_string(&map).unwrap();
    assert_eq!(
        serde_json::from_str::<BTreeMap<NodeId, String>>(&encoded).unwrap(),
        map
    );
    assert!(
        serde_json::from_str::<BTreeMap<NodeId, String>>(
            r#"{"00000000-0000-0000-0000-000000000000":"invalid"}"#
        )
        .is_err()
    );
}
