use super::*;

pub(super) async fn run(
    cli: &Cli,
    role: Role,
    signals: &Signals,
    revision: Revision,
    traffic: &mut Option<Traffic>,
) -> Result<()> {
    if !cli.reported_endpoints {
        return Ok(());
    }
    let before = running(cli, revision).await?;
    save(cli, "interface-before-status", &before).await?;
    signals
        .write(&format!("interface-ready-{}", role.name()), &Signal::Ready)
        .await?;
    for stage in ["changed", "restored"] {
        signals.ready(&format!("interface-state-{stage}")).await?;
        if let Some(traffic) = traffic {
            traffic.restored().await?;
            signals
                .write(&format!("interface-traffic-{stage}"), &Signal::Ready)
                .await?;
        }
        let after = running(cli, revision).await?;
        if after.process_id != before.process_id
            || after.identity != before.identity
            || after.revision != before.revision
            || !after.desired_up
        {
            return Err("live interface change replaced the daemon, identity or up intent".into());
        }
        save(cli, &format!("interface-{stage}-status"), &after).await?;
    }
    signals.ready("interface-complete").await?;
    Ok(())
}
