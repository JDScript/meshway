use super::*;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

#[tokio::test]
async fn writes_before_response_and_read_cancellation_preserves_partial_header() {
    let (local, mut remote) = tokio::io::duplex(32);
    let mut response = Response::new(local, Instant::now() + std::time::Duration::from_secs(2));
    response.write_all(b"request body").await.unwrap();
    response.flush().await.unwrap();
    let mut body = [0; 12];
    remote.read_exact(&mut body).await.unwrap();
    assert_eq!(&body, b"request body");
    remote.write_all(&[0]).await.unwrap();
    let mut output = [0xaa; 5];
    let mut read = Box::pin(response.read_exact(&mut output));
    assert!(futures_util::poll!(&mut read).is_pending());
    drop(read);
    assert_eq!(output, [0xaa; 5]);
    remote.write_all(b"\x00hello").await.unwrap();
    response.read_exact(&mut output).await.unwrap();
    assert_eq!(&output, b"hello");
}

#[tokio::test]
async fn rejects_version_addons_and_truncation_without_exposing_header() {
    for header in [vec![], vec![0], vec![1, 0], vec![0, 1, 0]] {
        let (local, mut remote) = tokio::io::duplex(16);
        remote.write_all(&header).await.unwrap();
        remote.shutdown().await.unwrap();
        let mut response = Response::new(local, Instant::now() + std::time::Duration::from_secs(2));
        let mut output = [0xaa; 4];
        assert!(response.read(&mut output).await.is_err());
        assert_eq!(output, [0xaa; 4]);
        assert!(response.read(&mut output).await.is_err());
        assert!(response.write_all(b"later").await.is_err());
    }
}

#[tokio::test]
async fn empty_read_is_noop_and_combined_header_payload_is_transparent() {
    let (local, mut remote) = tokio::io::duplex(32);
    let mut response = Response::new(local, Instant::now() + std::time::Duration::from_secs(2));
    assert_eq!(response.read(&mut []).await.unwrap(), 0);
    remote.write_all(b"\x00\x00hello").await.unwrap();
    remote.shutdown().await.unwrap();
    let mut output = Vec::new();
    response.read_to_end(&mut output).await.unwrap();
    assert_eq!(output, b"hello");
}

#[tokio::test]
async fn lazy_response_uses_original_deadline_and_drops_stalled_stream() {
    let (local, mut remote) = tokio::io::duplex(32);
    let mut response = Response::new(local, Instant::now());
    assert_eq!(
        response.read(&mut [0; 1]).await.unwrap_err().kind(),
        io::ErrorKind::TimedOut
    );
    assert_eq!(remote.read(&mut [0; 1]).await.unwrap(), 0);
}

#[tokio::test]
async fn expired_readable_header_is_rejected_before_any_payload_or_io() {
    for operation in 0..4 {
        let (local, mut remote) = tokio::io::duplex(32);
        remote.write_all(b"\x00\x00stale").await.unwrap();
        let mut response = Response::new(local, Instant::now());
        let mut output = [0xaa; 5];
        let error = match operation {
            0 => response.read(&mut output).await.unwrap_err(),
            1 => response.write(b"must not be sent").await.unwrap_err(),
            2 => response.flush().await.unwrap_err(),
            _ => response.shutdown().await.unwrap_err(),
        };
        assert_eq!(error.kind(), io::ErrorKind::TimedOut);
        assert_eq!(output, [0xaa; 5]);
        assert_eq!(remote.read(&mut [0; 32]).await.unwrap(), 0);
    }
}

#[tokio::test]
async fn one_way_writer_confirms_available_header_without_consuming_payload() {
    use std::time::Duration;

    let (local, mut remote) = tokio::io::duplex(32);
    let deadline = Instant::now() + Duration::from_millis(50);
    let mut response = Response::new(local, deadline);
    remote.write_all(b"\x00\x00reply").await.unwrap();
    response.write_all(b"one").await.unwrap();
    tokio::time::sleep_until(deadline).await;
    response.write_all(b"two").await.unwrap();
    let mut payload = [0; 5];
    response.read_exact(&mut payload).await.unwrap();
    assert_eq!(&payload, b"reply");
    let mut sent = [0; 6];
    remote.read_exact(&mut sent).await.unwrap();
    assert_eq!(&sent, b"onetwo");
}

#[tokio::test]
async fn deadline_wakes_flush_and_shutdown_without_a_reader() {
    use std::time::Duration;
    use tokio::io::BufStream;

    for shutdown in [false, true] {
        let (local, mut remote) = tokio::io::duplex(1);
        let mut response = Response::new(
            BufStream::with_capacity(1024, 1024, local),
            Instant::now() + Duration::from_millis(100),
        );
        response.write_all(&[7; 512]).await.unwrap();
        let result = tokio::time::timeout(Duration::from_secs(2), async {
            if shutdown {
                response.shutdown().await
            } else {
                response.flush().await
            }
        })
        .await
        .unwrap();
        assert_eq!(result.unwrap_err().kind(), io::ErrorKind::TimedOut);
        let mut drained = Vec::new();
        remote.read_to_end(&mut drained).await.unwrap();
        assert_eq!(drained, [7]);
    }
}

#[tokio::test]
async fn deadline_wakes_distinct_blocked_reader_and_writer_tasks() {
    use std::{future::poll_fn, time::Duration};
    use tokio::sync::oneshot;

    // Exercise both orders of registration in the clock's single waker slot.
    for writer_first in [false, true] {
        let (local, _remote) = tokio::io::duplex(1);
        let response = Response::new(local, Instant::now() + Duration::from_millis(200));
        let (mut reader, mut writer) = tokio::io::split(response);
        let (start_read, read_start) = oneshot::channel();
        let (read_pending, read_waiting) = oneshot::channel();
        let mut reading = tokio::spawn(async move {
            read_start.await.unwrap();
            let mut output = [0; 1];
            let read = reader.read(&mut output);
            tokio::pin!(read);
            let mut signal = Some(read_pending);
            poll_fn(|cx| {
                let result = read.as_mut().poll(cx);
                if result.is_pending()
                    && let Some(signal) = signal.take()
                {
                    let _ = signal.send(());
                }
                result
            })
            .await
        });
        let (start_write, write_start) = oneshot::channel();
        let (write_pending, write_waiting) = oneshot::channel();
        let mut writing = tokio::spawn(async move {
            write_start.await.unwrap();
            let write = writer.write_all(&[7; 32]);
            tokio::pin!(write);
            let mut signal = Some(write_pending);
            poll_fn(|cx| {
                let result = write.as_mut().poll(cx);
                if result.is_pending()
                    && let Some(signal) = signal.take()
                {
                    let _ = signal.send(());
                }
                result
            })
            .await
        });
        if writer_first {
            start_write.send(()).unwrap();
            write_waiting.await.unwrap();
            start_read.send(()).unwrap();
            read_waiting.await.unwrap();
        } else {
            start_read.send(()).unwrap();
            read_waiting.await.unwrap();
            start_write.send(()).unwrap();
            write_waiting.await.unwrap();
        }
        let result = tokio::time::timeout(Duration::from_secs(2), async {
            let read = (&mut reading).await.unwrap();
            let write = (&mut writing).await.unwrap();
            (read, write)
        })
        .await;
        if result.is_err() {
            reading.abort();
            writing.abort();
            let _ = reading.await;
            let _ = writing.await;
            panic!("the original deadline failed to wake both I/O tasks");
        }
        let (read, write) = result.unwrap();
        assert_eq!(read.unwrap_err().kind(), io::ErrorKind::TimedOut);
        assert_eq!(write.unwrap_err().kind(), io::ErrorKind::TimedOut);
    }
}
