use super::*;
use serde_json::json;
use utoipa::PartialSchema;

#[test]
fn positions_preserve_wire_fields_and_validate_every_construction_boundary() {
    for (latitude, longitude, radius) in [
        (-90.0, -180.0, None),
        (90.0, 180.0, Some(0.0)),
        (45.125, 120.5, Some(0.125)),
    ] {
        let position = GeoPosition::new(latitude, longitude, radius).unwrap();
        assert_eq!(position.latitude(), latitude);
        assert_eq!(position.longitude(), longitude);
        assert_eq!(position.accuracy_radius_km(), radius);
        let wire =
            json!({"latitude": latitude, "longitude": longitude, "accuracyRadiusKm": radius});
        assert_eq!(serde_json::to_value(&position).unwrap(), wire);
        assert_eq!(
            serde_json::from_value::<GeoPosition>(wire).unwrap(),
            position
        );
    }
    for (latitude, longitude, radius) in [
        (90.001, 0.0, None),
        (-90.001, 0.0, None),
        (0.0, 180.001, None),
        (0.0, -180.001, None),
        (0.0, 0.0, Some(-0.001)),
        (f64::NAN, 0.0, None),
        (0.0, f64::INFINITY, None),
        (0.0, 0.0, Some(f64::NAN)),
        (0.0, 0.0, Some(f64::INFINITY)),
    ] {
        assert!(GeoPosition::new(latitude, longitude, radius).is_err());
        // Only this private test can manufacture a corrupt internal value.
        // Serialization must not silently turn NaN/Infinity into JSON null.
        let corrupt = GeoPosition(GeoPositionFields {
            latitude,
            longitude,
            accuracy_radius_km: radius,
        });
        assert!(serde_json::to_string(&corrupt).is_err());
    }
    for wire in [
        r#"{"latitude":91,"longitude":0}"#,
        r#"{"latitude":0,"longitude":181}"#,
        r#"{"latitude":0,"longitude":0,"accuracyRadiusKm":-1}"#,
        r#"{"latitude":0,"longitude":0,"latitude":1}"#,
        r#"{"latitude":0,"longitude":0,"extra":1}"#,
        r#"[0,0,null]"#,
    ] {
        assert!(serde_json::from_str::<GeoPosition>(wire).is_err(), "{wire}");
    }
    // Existing optionality is retained; the change does not require a radius.
    assert!(serde_json::from_str::<GeoPosition>(r#"{"latitude":0,"longitude":0}"#).is_ok());
    let schema = serde_json::to_value(GeoPosition::schema()).unwrap();
    assert_eq!(GeoPosition::name(), "GeoPosition");
    assert_eq!(schema["required"], json!(["latitude", "longitude"]));
    assert_eq!(schema["properties"]["latitude"]["minimum"], -90);
    assert_eq!(schema["properties"]["longitude"]["maximum"], 180);
    assert_eq!(schema["additionalProperties"], false);
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct PlainPayload {
    label: String,
}

#[test]
fn buffered_lookup_payloads_keep_strict_objects_including_generic_consumers() {
    let base = json!({
        "status":"available",
        "datasetId":"00000000-0000-4000-8000-000000000001",
        "lookedUpAt":"2026-10-04T00:00:00Z",
        "data":{"label":"ok"}
    });
    let LookupResult::Available { data, .. } =
        serde_json::from_value::<LookupResult<PlainPayload>>(base.clone()).unwrap()
    else {
        panic!("available lookup")
    };
    assert_eq!(data.label, "ok");
    let mut wire = base;
    wire["data"] = json!(["positional"]);
    assert!(serde_json::from_value::<LookupResult<PlainPayload>>(wire.clone()).is_err());
    wire["data"] = json!([null, null, null, null]);
    assert!(serde_json::from_value::<LookupResult<GeoIpData>>(wire.clone()).is_err());
    wire["data"] = json!({"position":[0,0,null]});
    assert!(serde_json::from_value::<LookupResult<GeoIpData>>(wire.clone()).is_err());
    wire["data"] = json!([1, null, null]);
    assert!(serde_json::from_value::<LookupResult<AsnData>>(wire).is_err());
    // Re-entering strict decoding does not narrow the generic contract to objects.
    let scalar = r#"{"status":"available","datasetId":"00000000-0000-4000-8000-000000000001","lookedUpAt":"2026-10-04T00:00:00Z","data":42}"#;
    assert!(serde_json::from_str::<LookupResult<u32>>(scalar).is_ok());
    for wire in [
        r#"{"status":"available","datasetId":"00000000-0000-4000-8000-000000000001","lookedUpAt":"2026-10-04T00:00:00Z","data":{"country":"one","country":"two"}}"#,
        r#"{"status":"unavailable","reason":{"datasetUnavailable":null}}"#,
        r#"{"status":"unavailable","reason":"datasetUnavailable","reason":"lookupFailed"}"#,
        r#"{"status":"unavailable","reason":"datasetUnavailable","extra":1}"#,
    ] {
        assert!(serde_json::from_str::<LookupResult<GeoIpData>>(wire).is_err());
    }
}
