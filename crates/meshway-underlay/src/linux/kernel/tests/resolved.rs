//! The production resolved backend on the real systemd-resolved of an
//! isolated DNS image (`dev/dns-fixture/namespace.sh`), with the Kernel's
//! own DNS anchor: where resolved delivers a query to a link's loopback
//! server (253, or a stable release with systemd 5f9041af) it installs
//! beside other links' specific domains (DNS-2 D5); without it (Ubuntu
//! 22.04's 249.11) the probe fails naming that requirement (DNS-2 D4).
use super::*;
use futures_util::TryStreamExt;
use meshway_configuration::{DnsConfiguration, netmap::DnsPlan};
use meshway_dns::{SystemDnsError, SystemDnsOwner, SystemDnsPlan};

const FOREIGN: &str = "dns2-foreign";

fn resolvectl(arguments: &[&str]) {
    let status = std::process::Command::new("resolvectl")
        .args(arguments)
        .status()
        .unwrap();
    assert!(status.success(), "resolvectl {arguments:?}");
}

/// A foreign link as DHCP leaves it: up, addressed, with a search domain.
async fn foreign_link(handle: &Handle) -> u32 {
    handle
        .link()
        .add(rtnetlink::LinkDummy::new(FOREIGN).up().build())
        .execute()
        .await
        .unwrap();
    let index = handle
        .link()
        .get()
        .match_name(FOREIGN.to_owned())
        .execute()
        .try_next()
        .await
        .unwrap()
        .unwrap()
        .header
        .index;
    handle
        .address()
        .add(index, "192.0.2.77".parse().unwrap(), 24)
        .execute()
        .await
        .unwrap();
    index
}

fn plan(fallback: bool) -> SystemDnsPlan {
    let mut settings: DnsConfiguration =
        serde_json::from_str(r#"{"domain":"mesh.example"}"#).unwrap();
    settings
        .routes
        .insert("corp.example".into(), vec!["office".into()]);
    // `overrideLocal` installs the same fallback `~.` as an exit plan.
    settings.override_local = fallback;
    SystemDnsPlan::new(&settings, &DnsPlan::NoExit).unwrap()
}

#[tokio::test(flavor = "current_thread")]
#[ignore = "requires the isolated DNS image with its own D-Bus and systemd-resolved, NET_ADMIN and private /run"]
async fn resolved_backend_installs_beside_foreign_domains_or_names_its_systemd_requirement() {
    assert_eq!(std::env::var("MESHWAY_DNS_ISOLATED").as_deref(), Ok("1"));
    let installs = match std::env::var("MESHWAY_RESOLVED_EXPECT").as_deref() {
        Ok("installs") => true,
        Ok("unsupported") => false,
        other => panic!("MESHWAY_RESOLVED_EXPECT must be installs or unsupported: {other:?}"),
    };
    let (connection, handle) = rtnl::connect().unwrap();
    let connection = tokio::spawn(connection);
    let foreign = foreign_link(&handle).await;
    resolvectl(&["domain", FOREIGN, "tko.oraclevcn.com"]);
    let temp = tempfile::tempdir().unwrap();
    let directory = Arc::new(Directory::open(&temp.path().join("node"), "node.lock").unwrap());
    let reservation = SystemDnsOwner::reserve(Default::default(), directory.clone())
        .await
        .unwrap();
    let tasks = Tasks::new(64);
    let mut kernel = Kernel::open(directory.clone(), Default::default(), tasks.clone())
        .await
        .unwrap();
    let anchor = kernel.dns_anchor(&reservation).await.unwrap();
    let mut owner = SystemDnsOwner::prepare_reserved(reservation, Some(anchor))
        .await
        .unwrap();
    let fallback = plan(true);
    if !installs {
        assert_eq!(
            owner.install(&fallback).await,
            Err(SystemDnsError::Unsupported)
        );
        let failure = owner.failure().unwrap().clone();
        println!("{failure}");
        assert_eq!(failure.check, "probe");
        assert!(
            failure.expected.as_deref().is_some_and(
                |expected| expected.contains("systemd-resolved 253 or later, or stable 249.15")
            ),
            "{failure}"
        );
        assert!(
            failure
                .actual
                .as_deref()
                .is_some_and(|actual| actual.starts_with("no query reached 127.0.0.1:")),
            "{failure}"
        );
    } else {
        // D5: the fallback yields OCI's search domain to its link.
        owner.install(&fallback).await.unwrap();
        let local: Vec<_> = owner
            .local_domains()
            .iter()
            .map(|local| {
                (
                    local.domain.clone(),
                    local.link_index,
                    local.link_name.clone(),
                )
            })
            .collect();
        assert_eq!(
            local,
            [(
                "tko.oraclevcn.com".to_owned(),
                foreign,
                Some(FOREIGN.to_owned())
            )]
        );
        owner.observe().await.unwrap();
        // A foreign domain under an explicit route, or a second `~.`, is a
        // conflict; once it is gone the owner re-proves and installs again.
        for conflict in ["~a.corp.example", "~."] {
            resolvectl(&["domain", FOREIGN, "tko.oraclevcn.com", conflict]);
            assert_eq!(owner.observe().await, Err(SystemDnsError::Conflict));
            let failure = owner.failure().unwrap().clone();
            println!("{failure}");
            assert_eq!(failure.check, "foreign_domain", "{failure}");
            resolvectl(&["domain", FOREIGN, "tko.oraclevcn.com"]);
            owner.recover().await.unwrap();
            owner.install(&fallback).await.unwrap();
            assert_eq!(owner.local_domains().len(), 1);
        }
        // A pure Mesh plan has no fallback: a foreign `~.` routes nothing
        // it covers, and nothing is reported.
        owner.install(&plan(false)).await.unwrap();
        resolvectl(&["domain", FOREIGN, "tko.oraclevcn.com", "~."]);
        owner.observe().await.unwrap();
        assert!(owner.local_domains().is_empty());
    }
    owner.down().await.unwrap();
    kernel.remove_dns_anchor().await.unwrap();
    kernel.down().await.unwrap();
    kernel.settle().await.unwrap();
    drop(kernel);
    tasks.shutdown().await.unwrap();
    handle.link().del(foreign).execute().await.unwrap();
    connection.abort();
}
