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
    let app = App::open(Settings::new(&data)).await.unwrap();
    let status = app.ip_datasets().status().unwrap();
    assert!(status.geoip.is_some() && status.asn.is_some());
    assert!(installed.contains(&status.geoip.unwrap().to_string()));
    let info = app.ip_datasets().lookup(documented.into()).unwrap();
    let LookupResult::Available {
        data, dataset_id, ..
    } = info.geoip.clone()
    else {
        panic!("{info:?}")
    };
    assert_eq!(dataset_id, status.geoip.unwrap());
    assert_eq!(data.country.as_deref(), Some("JP"));
    let LookupResult::Available { data, .. } = info.asn.clone() else {
        panic!("{info:?}")
    };
    assert_eq!(data.number, 64512);
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
}
