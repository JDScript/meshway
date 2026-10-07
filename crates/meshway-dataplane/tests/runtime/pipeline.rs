use super::{datagram, support::secure::SecureMesh};
use meshway_dataplane::secure::{
    Error, Received,
    pipeline::{self, Job, Output, Pool},
};
use std::collections::VecDeque;

#[tokio::test]
async fn parallel_crypto_preserves_order_and_commits_replay_only_after_verification() {
    let mut fixture = SecureMesh::start().await;
    let circuit = fixture.handshake().await;
    for workers in [0, 3] {
        let mut pool = Pool::new(workers).unwrap();
        let mut jobs = VecDeque::new();
        for index in 0usize..32 {
            let mut packet = datagram(&fixture.mesh, false, false);
            if index.is_multiple_of(2) {
                let traffic = packet.facts().traffic;
                packet = super::packet::udp(
                    traffic.source,
                    traffic.destination,
                    50000,
                    42424,
                    &[7; 512],
                );
            }
            jobs.push_back(Job::Send(
                fixture
                    .a
                    .prepare_send(circuit, packet.into_buffer())
                    .unwrap(),
            ));
        }
        let mut previous = None;
        while let Some(job) = jobs.pop_front() {
            assert!(pool.submit(job).is_none());
        }
        if workers != 0 {
            // Small jobs have completed inline, but cannot pass the first large
            // job, which has not even been dispatched to a worker yet.
            assert!(pool.take_ready().is_empty());
        }
        pool.flush();
        let mut results = Vec::new();
        while pool.has_pending() {
            pool.ready().await;
            results.extend(pool.take_ready());
        }
        assert_eq!(results.len(), 32);
        for result in results {
            let (result, _credit) = result.into_parts();
            let Output::Send(frame) = result.unwrap() else {
                panic!("send output")
            };
            let wire = frame.release(|bytes| bytes).unwrap();
            let sequence = u64::from_be_bytes(wire[42..50].try_into().unwrap());
            if let Some(previous) = previous {
                assert_eq!(sequence, previous + 1);
            }
            previous = Some(sequence);
            let duplicate = wire.payload().to_vec();
            let mut tampered = duplicate.clone();
            *tampered.last_mut().unwrap() ^= 1;
            let invalid = fixture
                .r
                .prepare_receive(&fixture.ar.carrier, tampered.into())
                .unwrap();
            assert!(matches!(pipeline::open(invalid), Err(Error::Proof)));
            let first = fixture
                .r
                .prepare_receive(&fixture.ar.carrier, wire)
                .unwrap();
            let second = fixture
                .r
                .prepare_receive(&fixture.ar.carrier, duplicate.clone().into())
                .unwrap();
            let first = pipeline::open(first).unwrap();
            let second = pipeline::open(second).unwrap();
            let Received::Forward(frame) = fixture.r.commit_receive(first).unwrap() else {
                panic!("relay")
            };
            assert!(matches!(
                fixture.r.commit_receive(second),
                Err(Error::Replay)
            ));
            assert!(matches!(
                fixture
                    .r
                    .prepare_receive(&fixture.ar.carrier, duplicate.into()),
                Err(Error::Replay)
            ));
            let job = fixture
                .b
                .prepare_receive(&fixture.rb.carrier, frame.release(|bytes| bytes).unwrap())
                .unwrap();
            let Received::Packet(packet) = fixture
                .b
                .commit_receive(pipeline::open(job).unwrap())
                .unwrap()
            else {
                panic!("endpoint")
            };
            packet.release(|_| ()).unwrap();
        }
        assert!(jobs.is_empty());
    }
    let packet = datagram(&fixture.mesh, false, false);
    let job = fixture
        .a
        .prepare_send(circuit, packet.into_buffer())
        .unwrap();
    fixture.a.close(circuit);
    // Crypto completion is not authority: worker can finish, release must fail.
    assert!(pipeline::seal(job).unwrap().release(|_| ()).is_err());
    fixture.close().await;
}

#[tokio::test]
async fn closing_between_open_and_commit_never_delivers_or_advances_owner() {
    let mut fixture = SecureMesh::start().await;
    let circuit = fixture.handshake().await;
    let packet = datagram(&fixture.mesh, false, false);
    let frame = fixture.a.send(circuit, packet).unwrap();
    let job = fixture
        .r
        .prepare_receive(&fixture.ar.carrier, frame.release(|bytes| bytes).unwrap())
        .unwrap();
    let opened = pipeline::open(job).unwrap();
    fixture.r.close(circuit);
    assert!(fixture.r.commit_receive(opened).is_err());
    assert_eq!(fixture.r.retained_flows(), 0);
    fixture.close().await;
}
