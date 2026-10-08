use super::*;

#[tokio::test]
async fn datagram_receive_coalesces_ready_scratch_prefixes_and_defers_error() {
    struct Endpoint(Mutex<VecDeque<io::Result<Vec<u8>>>>);
    impl crate::raw::Datagram for Endpoint {
        fn local_addr(&self) -> io::Result<SocketAddr> {
            Ok(peer())
        }
        fn poll_send_to(
            &self,
            _: &mut Context<'_>,
            _: &[u8],
            _: SocketAddr,
        ) -> Poll<io::Result<usize>> {
            Poll::Pending
        }
        fn poll_recv_from(
            &self,
            _: &mut Context<'_>,
            bytes: &mut [u8],
        ) -> Poll<io::Result<crate::raw::DatagramRead>> {
            let Some(result) = self.0.lock().unwrap().pop_front() else {
                return Poll::Pending;
            };
            let packet = result?;
            bytes[..packet.len()].copy_from_slice(&packet);
            Poll::Ready(Ok(crate::raw::DatagramRead {
                source: peer(),
                copied_len: packet.len(),
                original_len: Some(packet.len()),
                truncated: false,
            }))
        }
    }
    // One scratch buffer under this storage budget, four-frame public batch.
    let limits = Limits::new(65536, 4, 65536, 1, 4).unwrap();
    let mut input: VecDeque<_> = (0..5).map(|n| Ok(vec![n; 16])).collect();
    input.push_back(Err(io::ErrorKind::ConnectionRefused.into()));
    input.push_back(Ok(vec![9; 16]));
    let mut session = Session::datagram(Endpoint(Mutex::new(input)), limits);
    let first = session.receiver().recv_batch().await.unwrap();
    assert_eq!(
        first.iter().map(|f| f.payload[0]).collect::<Vec<_>>(),
        [0, 1, 2, 3]
    );
    let prefix = session.receiver().recv_batch().await.unwrap();
    assert_eq!(prefix.len(), 1);
    assert_eq!(prefix[0].payload[0], 4);
    assert!(
        matches!(session.receiver().recv().await, Err(ReceiveError::Io(error)) if error.kind() == io::ErrorKind::ConnectionRefused)
    );
    assert_eq!(
        session.receiver().recv_batch().await.unwrap()[0].payload[0],
        9
    );
    session.shutdown().await.unwrap();
}

#[tokio::test]
async fn stream_receive_batch_keeps_partial_codec_and_delivers_prefix_before_eof() {
    let (stream, mut remote) = tokio::io::duplex(128);
    let mut session = Session::stream(stream, peer(), limits());
    remote.write_all(&[0, 0]).await.unwrap();
    {
        let mut read = Box::pin(session.receiver().recv_batch());
        assert!(futures_util::poll!(&mut read).is_pending());
    }
    let encoded = wire(&[b"one", b"two"]);
    remote.write_all(&encoded[2..]).await.unwrap();
    remote.shutdown().await.unwrap();
    let frames = session.receiver().recv_batch().await.unwrap();
    assert_eq!(
        frames
            .iter()
            .map(|frame| frame.payload.as_slice())
            .collect::<Vec<_>>(),
        [b"one", b"two"]
    );
    assert!(matches!(
        session.receiver().recv().await,
        Err(ReceiveError::Closed)
    ));
    session.shutdown().await.unwrap();
}

#[tokio::test]
async fn stream_receive_batch_preserves_next_frame_at_byte_boundary() {
    let (stream, mut remote) = tokio::io::duplex(100_000);
    let mut session = Session::stream(
        stream,
        peer(),
        Limits::new(50_000, 3, 150_000, 1, 3).unwrap(),
    );
    remote
        .write_all(&wire(&[&vec![1; 40_000], &vec![2; 40_000]]))
        .await
        .unwrap();
    let frames = session.receiver().recv_batch().await.unwrap();
    assert_eq!(frames.len(), 1);
    assert_eq!(frames[0].payload, vec![1; 40_000]);
    assert_eq!(
        session.receiver().recv().await.unwrap().payload,
        vec![2; 40_000]
    );
    session.shutdown().await.unwrap();
}

#[tokio::test]
async fn receive_is_cancellation_safe_across_partial_header_and_payload() {
    let (stream, mut remote) = tokio::io::duplex(128);
    let mut session = Session::stream(stream, peer(), limits());
    let input = wire(&[b"hello", b"world"]);
    remote.write_all(&input[..2]).await.unwrap();
    let mut receive = Box::pin(session.receiver().recv());
    assert!(futures_util::poll!(&mut receive).is_pending());
    drop(receive);
    remote.write_all(&input[2..7]).await.unwrap();
    let mut receive = Box::pin(session.receiver().recv());
    assert!(futures_util::poll!(&mut receive).is_pending());
    drop(receive);
    remote.write_all(&input[7..]).await.unwrap();
    assert_eq!(session.receiver().recv().await.unwrap().payload, b"hello");
    assert_eq!(session.receiver().recv().await.unwrap().payload, b"world");
    session.shutdown().await.unwrap();
}

#[tokio::test]
async fn malformed_stream_lengths_close_carrier_without_reading_body() {
    for header in [u32::MAX.to_be_bytes(), 0_u32.to_be_bytes()] {
        let (stream, mut remote) = tokio::io::duplex(16);
        let mut session = Session::stream(stream, peer(), limits());
        remote.write_all(&header).await.unwrap();
        // A maximal bogus header must fail immediately with only a header
        // present: no oversized allocation or wait for an impossible body.
        let result = timeout(Duration::from_secs(2), session.receiver().recv())
            .await
            .unwrap();
        if header == u32::MAX.to_be_bytes() {
            assert!(matches!(result, Err(ReceiveError::Oversized)));
        } else {
            assert!(matches!(result, Err(ReceiveError::Empty)));
        }
        assert!(session.sender().is_closed());
        assert!(matches!(
            session.receiver().recv().await,
            Err(ReceiveError::Closed)
        ));
        session.shutdown().await.unwrap();
    }
}

#[tokio::test]
async fn incomplete_frame_eof_is_not_a_complete_frame() {
    let (stream, mut remote) = tokio::io::duplex(32);
    let mut session = Session::stream(stream, peer(), limits());
    remote.write_all(&[0, 0, 0, 3, 1]).await.unwrap();
    remote.shutdown().await.unwrap();
    assert!(matches!(
        session.receiver().recv().await,
        Err(ReceiveError::Io(_))
    ));
    assert!(session.sender().is_closed());
    session.shutdown().await.unwrap();
}

#[tokio::test]
async fn bounded_read_ahead_decodes_coalesced_frames_from_one_raw_read() {
    let configured = Limits::new(128, 8, 512 * 1024, 2, 8).unwrap();
    assert_eq!(configured.stream_buffer_bytes(), 256 * 1024);
    let (observed, mut remote, observation) = observed_stream(512);
    let mut session = Session::stream(observed, peer(), configured);
    remote.write_all(&wire(&[b"one", b"two"])).await.unwrap();
    assert_eq!(session.receiver().recv().await.unwrap().payload, b"one");
    assert_eq!(session.receiver().recv().await.unwrap().payload, b"two");
    assert_eq!(observation.lock().unwrap().read_requests, [64 * 1024]);
    session.shutdown().await.unwrap();
}
