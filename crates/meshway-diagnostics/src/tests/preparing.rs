use super::*;
use meshway_dataplane::secure::Received;

/// Deliver the actual protected wire through the final synchronous sink. This
/// fixture has no simulated cipher, permit, handshake or alternative flow owner.
fn deliver(
    mut frame: meshway_dataplane::secure::PendingFrame,
    path: &[NodeId],
    nodes: &mut [SecureNode],
    owner: &mut Owner,
    channel: &ProbeChannel,
) {
    for _ in 0..16 {
        let next = path.iter().position(|node| *node == frame.next()).unwrap();
        let carrier = frame.carrier().clone();
        let bytes = frame.release(std::convert::identity).unwrap();
        frame = match nodes[next].receive(&carrier, bytes).unwrap() {
            Received::Forward(next) | Received::ProbeRequest { response: next, .. } => next,
            Received::Established {
                response: Some(next),
                ..
            } => next,
            Received::Established { response: None, .. } => return,
            Received::ProbeReply {
                circuit,
                measurement_id,
                challenge,
            } => {
                owner.reply(
                    circuit,
                    measurement_id,
                    challenge,
                    channel.clone(),
                    Instant::now(),
                );
                return;
            }
            Received::Packet(_) | Received::DnsRequest(_) | Received::DnsReply(_) => {
                panic!("diagnostic produced other traffic")
            }
        };
    }
    panic!("protected exchange exceeded its complete path budget");
}

#[tokio::test]
async fn ready_channel_and_handshake_events_start_direct_and_relay_probes_without_ticks() {
    for relay in [false, true] {
        let mut fixture = Fixture::open_route(relay).await;
        let path = fixture.owner.tasks[&fixture.task.id].path.clone();
        assert_eq!(path.len(), if relay { 3 } else { 2 });
        let mut carriers = vec![BTreeMap::new(); path.len()];
        for (index, edge) in path.windows(2).enumerate() {
            let carrier = Carrier {
                dialer: edge[0],
                listener: edge[1],
                listener_key: "udp".into(),
                protocol: meshway_configuration::TransportProtocol::NativeUdp,
            };
            carriers[index].insert(edge[1], carrier.clone());
            carriers[index + 1].insert(edge[0], carrier);
        }
        let mut nodes = Vec::new();
        for (node, carriers) in path.iter().zip(&carriers) {
            let index = fixture
                .controls
                .iter()
                .position(|control| control.status().binding.unwrap().node_id == *node)
                .unwrap();
            nodes.push(fixture.secure(index, carriers.clone()));
        }
        // Initial installation may precede authenticated adjacency. It neither
        // manufactures a measurement nor schedules a periodic acquisition poll.
        assert!(
            fixture
                .owner
                .actions(&mut nodes[0], &BTreeMap::new(), MAX_TASKS)
                .is_empty()
        );
        let channel = ProbeChannel {
            peer_node_id: path[1],
            ..fixture.channel()
        };
        // One actual carrier event must initiate the handshake immediately.
        let action = fixture
            .owner
            .actions(&mut nodes[0], &carriers[0], MAX_TASKS)
            .pop()
            .expect("ready carrier must not wait for a one-second setup tick");
        assert!(action.measurement_id.is_none());
        fixture
            .owner
            .complete_send(fixture.task.id, None, Ok((channel.clone(), Instant::now())));
        deliver(
            action.frame,
            &path,
            &mut nodes,
            &mut fixture.owner,
            &channel,
        );
        // The received authenticated handshake completion is already an event;
        // it must not wait for the handshake retransmission timer or a 100 ms poll.
        let action = fixture
            .owner
            .actions(&mut nodes[0], &carriers[0], MAX_TASKS)
            .pop()
            .expect("established path must immediately admit its first probe");
        let measurement = action.measurement_id.expect("first measurement");
        // Another subscriber may reuse this diagnostic circuit without an
        // in-flight measurement. Its earlier map key must not steal a reply
        // belonging to the original task's exact (CID, measurement, challenge).
        let mut idle = fixture.task.clone();
        idle.id = meshway_model::ObservationId::new(uuid::Uuid::from_u128(1)).unwrap();
        let (desired, bound) = fixture.content(0);
        fixture
            .owner
            .synchronize(&desired, bound, &[idle.clone(), fixture.task.clone()])
            .unwrap();
        assert!(
            fixture
                .owner
                .actions(&mut nodes[0], &carriers[0], MAX_TASKS)
                .is_empty()
        );
        assert_eq!(
            fixture.owner.tasks[&idle.id].circuit,
            Some(action.frame.circuit())
        );
        let sent = Instant::now();
        deliver(
            action.frame,
            &path,
            &mut nodes,
            &mut fixture.owner,
            &channel,
        );
        fixture
            .owner
            .complete_send(fixture.task.id, Some(measurement), Ok((channel, sent)));
        assert_eq!(
            fixture.sample().probe.as_ref().unwrap().outcome,
            ProbeOutcome::Success
        );
        let limit = Duration::from_millis(if relay { 2500 } else { 1500 });
        assert!(fixture.accepted_at.duration_since(fixture.created_at) < limit);
        assert!(fixture.installed_at.duration_since(fixture.accepted_at) < limit);
        assert!(
            fixture.created_at.elapsed() < limit,
            "event-driven setup exceeded {limit:?}"
        );
        // The same real protected completion crosses the generated HTTP API
        // and the Server's history transaction. History cannot discard the
        // task's already-approved complete path or reject every path sample.
        let output = fixture.owner.outputs().next().unwrap().clone();
        let accepted = fixture.controls[0]
            .report_observation_result(&fixture.task, &output.result)
            .await
            .unwrap();
        assert_eq!(accepted.latest.as_ref(), Some(fixture.sample()));
        let replay = fixture.controls[0]
            .report_observation_result(&fixture.task, &output.result)
            .await
            .unwrap();
        assert_eq!(replay.latest, accepted.latest);
        assert_eq!(replay.path_segment, accepted.path_segment);
        assert_eq!(replay.phase, accepted.phase);
        fixture.owner.acknowledge(fixture.task.id);
        assert!(
            fixture
                .owner
                .actions(&mut nodes[0], &carriers[0], MAX_TASKS)
                .is_empty(),
            "first-probe acceleration must preserve the one-second measurement pace"
        );
        let next = fixture.owner.tasks[&fixture.task.id].next;
        fixture.owner.suspend();
        let (desired, bound) = fixture.content(0);
        fixture
            .owner
            .synchronize(&desired, bound, std::slice::from_ref(&fixture.task))
            .unwrap();
        assert_eq!(
            fixture.owner.tasks[&fixture.task.id].next, next,
            "handoff must retain the prior probe's pacing deadline"
        );
        let task = fixture.owner.tasks.get_mut(&fixture.task.id).unwrap();
        task.preparing = Instant::now() - SETUP_TIMEOUT;
        task.next = Instant::now();
        assert!(
            fixture
                .owner
                .actions(&mut nodes[0], &carriers[0], MAX_TASKS)
                .is_empty(),
            "a late ready-channel event must not outrun the original setup deadline"
        );
        assert!(matches!(
            fixture.owner.outputs().next().unwrap().result.result,
            ObservationUpdate::Phase {
                phase: ReportedObservationPhase::Failed
            }
        ));
        fixture.close().await;
    }
}
