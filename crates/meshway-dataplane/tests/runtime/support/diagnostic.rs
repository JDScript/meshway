use super::secure::SecureMesh;
use meshway_dataplane::secure::CircuitAuthorization;
use meshway_protocol::ObservationTask;

pub async fn setup() -> (SecureMesh, ObservationTask) {
    let mut fixture = SecureMesh::start().await;
    fixture
        .mesh
        .server
        .configure(&fixture.mesh.owner, |cfg| {
            cfg.access[0].ip.push("icmp:*".into())
        })
        .await;
    fixture.reconcile().await;
    let task = fixture
        .mesh
        .server
        .observe(&fixture.mesh.owner, fixture.mesh.a.id, fixture.mesh.b.id)
        .await;
    fixture.reconcile().await;
    for node in [&fixture.mesh.a, &fixture.mesh.r, &fixture.mesh.b] {
        let snapshot = node.control.execution_snapshot().unwrap();
        let cfg = snapshot.configuration().unwrap();
        assert_eq!(cfg.diagnostics[&task.id].paths[0].nodes, fixture.mesh.path);
        assert_eq!(cfg.permits.len(), 1);
        assert_eq!(
            cfg.diagnostics[&task.id].instance_id,
            fixture.mesh.a.instance().instance_id
        );
    }
    (fixture, task)
}

pub async fn handshake(fixture: &mut SecureMesh, task: &ObservationTask) -> [u8; 32] {
    let init = fixture
        .a
        .start_authorized(
            CircuitAuthorization::Diagnostic(task.id),
            fixture.mesh.path.clone(),
            &fixture.ar.carrier,
        )
        .unwrap();
    fixture.finish_handshake(init).await
}

pub async fn renew_observer(fixture: &mut SecureMesh, task: &mut ObservationTask) {
    // Renew only at boundaries with no queued old-publication outputs under
    // test, then install the actual successor before creating another probe.
    if fixture
        .mesh
        .server
        .renew_observation_if_due(&fixture.mesh.owner, task)
        .await
    {
        fixture.reconcile().await;
    }
}
