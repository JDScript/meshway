//! The host-maintenance command installs GeoIP / ASN datasets; a Server that
//! opens the same storage later has them from its first report on.
mod support;

use meshway_model::resources::LookupResult;
use meshway_server::resources::datasets::IpDatasetKind;
use meshway_server::{App, Settings};
use std::{net::Ipv4Addr, path::Path, process::Command};
use support::mmdb;

fn run(data: &Path, args: &[&str]) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_ip-datasets"))
        .args(["--log-format", "json", "--log-filter", "info"])
        .arg("--data-dir")
        .arg(data)
        .args(args)
        .output()
        .unwrap()
}

#[tokio::test]
async fn imported_datasets_survive_restart_and_answer_lookups() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().canonicalize().unwrap();
    let data = root.join("data");
    // A public address; documentation ranges are "not applicable" by design.
    let documented = Ipv4Addr::new(93, 184, 216, 34);
    let city = root.join("GeoLite2-City.mmdb");
    std::fs::write(
        &city,
        mmdb::database(
            "GeoLite2-City",
            &[(
                Ipv4Addr::new(93, 184, 216, 0),
                24,
                mmdb::city("JP", "Tokyo", 35.7, 139.7),
            )],
        ),
    )
    .unwrap();
    let asn = root.join("GeoLite2-ASN.mmdb");
    std::fs::write(
        &asn,
        mmdb::database(
            "GeoLite2-ASN",
            &[(
                Ipv4Addr::new(93, 184, 216, 0),
                24,
                mmdb::asn(64512, "Example Networks"),
            )],
        ),
    )
    .unwrap();

    // Settings problems are reported as such, not as another instance.
    let output = run(Path::new("relative/data"), &["status"]);
    assert!(!output.status.success());
    let stderr = String::from_utf8(output.stderr).unwrap();
    assert!(
        stderr.contains(
            "Server storage startup failed: data directory must be an absolute path without parent traversal."
        ),
        "{stderr}"
    );
    assert!(!stderr.contains("Stop the Server first"));

    // Nothing installed: status says so and lookups are honest about it.
    let output = run(&data, &["status"]);
    assert!(output.status.success(), "{output:?}");
    assert_eq!(
        String::from_utf8(output.stdout).unwrap(),
        "geoip: none\nasn: none\n"
    );

    // The kind is checked against the database type, not the file name.
    let output = run(
        &data,
        &["import", "--kind", "geoip", "--file", asn.to_str().unwrap()],
    );
    assert!(!output.status.success());
    assert!(output.stdout.is_empty());

    let output = run(
        &data,
        &[
            "import",
            "--kind",
            "geoip",
            "--file",
            city.to_str().unwrap(),
        ],
    );
    assert!(output.status.success(), "{output:?}");
    let installed = String::from_utf8(output.stdout).unwrap();
    assert!(
        installed.starts_with("Installed GeoIp dataset "),
        "{installed}"
    );
    let output = run(
        &data,
        &["import", "--kind", "asn", "--file", asn.to_str().unwrap()],
    );
    assert!(output.status.success(), "{output:?}");

    let output = run(&data, &["lookup", &documented.to_string()]);
    assert!(output.status.success(), "{output:?}");
    assert_eq!(
        String::from_utf8(output.stdout).unwrap(),
        "geoip: country=JP city=Tokyo position=35.7,139.7\nasn: AS64512 Example Networks\n"
    );
    let output = run(&data, &["lookup", "93.184.215.1"]);
    assert_eq!(
        String::from_utf8(output.stdout).unwrap(),
        "geoip: not in dataset\nasn: not in dataset\n"
    );
    for private in ["10.0.0.1", "203.0.113.7"] {
        let output = run(&data, &["lookup", private]);
        assert!(
            String::from_utf8(output.stdout)
                .unwrap()
                .starts_with("geoip: not applicable")
        );
    }

    // A Server opening the storage loads both datasets before serving.
    let app = App::open_with_connector(
        Settings::new(&data),
        std::sync::Arc::new(meshway_server::resources::download::Offline),
    )
    .await
    .unwrap();
    let status = app.ip_datasets().status().unwrap();
    assert!(status.geoip.is_some() && status.asn.is_some());
    assert!(installed.contains(&status.geoip.unwrap().to_string()));
    let info = app.ip_datasets().lookup(documented.into()).unwrap();
    let LookupResult::Available {
        data: geoip,
        dataset_id,
        ..
    } = info.geoip.clone()
    else {
        panic!("{info:?}")
    };
    assert_eq!(dataset_id, status.geoip.unwrap());
    assert_eq!(geoip.country.as_deref(), Some("JP"));
    let LookupResult::Available { data: asn, .. } = info.asn.clone() else {
        panic!("{info:?}")
    };
    assert_eq!(asn.number, 64512);
    // Re-importing replaces the current dataset; lookups cite the new artifact.
    let again = app
        .ip_datasets()
        .install(
            IpDatasetKind::GeoIp,
            "second".into(),
            std::fs::read(&city).unwrap(),
        )
        .await
        .unwrap();
    assert_ne!(Some(again), status.geoip);
    assert_eq!(app.ip_datasets().status().unwrap().geoip, Some(again));
    app.shutdown().await.unwrap();
    let app = App::open_with_connector(
        Settings::new(&data),
        std::sync::Arc::new(meshway_server::resources::download::Offline),
    )
    .await
    .unwrap();
    assert_eq!(app.ip_datasets().status().unwrap().geoip, Some(again));
    assert!(
        matches!(app.ip_datasets().lookup(documented.into()).unwrap().geoip,
        LookupResult::Available { dataset_id, .. } if dataset_id == again)
    );
    app.shutdown().await.unwrap();
}

#[tokio::test]
async fn invalid_mmdb_coordinates_fail_the_lookup_without_poisoning_asn_or_wire_data() {
    use meshway_model::resources::{IpInfo, UnavailableReason};
    let temp = tempfile::tempdir().unwrap();
    let data = temp.path().canonicalize().unwrap().join("data");
    let app = App::open_with_connector(
        Settings::new(&data),
        std::sync::Arc::new(meshway_server::resources::download::Offline),
    )
    .await
    .unwrap();
    let prefix = Ipv4Addr::new(93, 184, 216, 0);
    let address = Ipv4Addr::new(93, 184, 216, 34);
    let asn_id = app
        .ip_datasets()
        .install(
            IpDatasetKind::Asn,
            "valid ASN".into(),
            mmdb::database("GeoLite2-ASN", &[(prefix, 24, mmdb::asn(64512, "Example"))]),
        )
        .await
        .unwrap();
    for (latitude, longitude) in [
        (91.0, 0.0),
        (-91.0, 0.0),
        (0.0, 181.0),
        (0.0, -181.0),
        (f64::NAN, 0.0),
        (0.0, f64::INFINITY),
        (f64::NEG_INFINITY, 0.0),
    ] {
        // These are real, parseable MMDB records, not forged GeoPosition DTOs.
        app.ip_datasets()
            .install(
                IpDatasetKind::GeoIp,
                "invalid coordinate record".into(),
                mmdb::database(
                    "GeoLite2-City",
                    &[(prefix, 24, mmdb::city("JP", "Tokyo", latitude, longitude))],
                ),
            )
            .await
            .unwrap();
        let info = app.ip_datasets().lookup(address.into()).unwrap();
        assert!(matches!(
            info.geoip,
            LookupResult::Unavailable {
                reason: UnavailableReason::LookupFailed
            }
        ));
        assert!(
            matches!(info.asn, LookupResult::Available { dataset_id, .. } if dataset_id == asn_id)
        );
        let encoded = serde_json::to_vec(&info).unwrap();
        assert_eq!(serde_json::from_slice::<IpInfo>(&encoded).unwrap(), info);
        assert_eq!(app.ip_datasets().lookup(address.into()).unwrap(), info);
    }
    app.ip_datasets()
        .install(
            IpDatasetKind::GeoIp,
            "valid coordinate boundaries".into(),
            mmdb::database(
                "GeoLite2-City",
                &[(prefix, 24, mmdb::city("JP", "Tokyo", -90.0, 180.0))],
            ),
        )
        .await
        .unwrap();
    let info = app.ip_datasets().lookup(address.into()).unwrap();
    assert!(
        matches!(info.geoip, LookupResult::Available { ref data, .. } if data.position.is_some())
    );
    assert_eq!(
        serde_json::from_slice::<IpInfo>(&serde_json::to_vec(&info).unwrap()).unwrap(),
        info
    );
    app.shutdown().await.unwrap();
}
