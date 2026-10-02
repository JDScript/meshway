use super::*;

#[test]
fn rejects_wrong_transaction_question_type_trailing_and_malformed_response() {
    let (name, kind) = question("node.meshway.test", None).unwrap();
    let mut request = Message::new(123, MessageType::Query, OpCode::Query);
    request.add_query(Query::query(name, kind.record_type()));
    let mut reply = Message::response(123, OpCode::Query);
    reply.queries = request.queries.clone();
    assert!(validate_response(&request, &reply.to_vec().unwrap()).is_ok());
    let mut wrong = reply.clone();
    wrong.metadata.id = 124;
    assert!(validate_response(&request, &wrong.to_vec().unwrap()).is_err());
    let mut wrong = reply.clone();
    wrong.queries[0] = Query::query(Name::from_ascii("evil.test").unwrap(), RecordType::A);
    assert!(validate_response(&request, &wrong.to_vec().unwrap()).is_err());
    let mut wrong = reply.clone();
    wrong.queries[0] = Query::query(request.queries[0].name().clone(), RecordType::AAAA);
    assert!(validate_response(&request, &wrong.to_vec().unwrap()).is_err());
    let mut bytes = reply.to_vec().unwrap();
    bytes.push(0);
    assert!(validate_response(&request, &bytes).is_err());
    assert!(validate_response(&request, &[0; 5]).is_err());
    assert!(question("100.64.0.2", Some(QueryType::A)).is_err());
}

#[tokio::test]
async fn malicious_udp_oversize_wrong_endpoint_and_timeout_are_failures() {
    for behavior in [0, 1, 2, 3, 4] {
        let socket = UdpSocket::bind("127.0.0.1:0").await.unwrap();
        let endpoint = socket.local_addr().unwrap();
        let mut metadata = Status::unavailable();
        metadata.listener = Some(endpoint);
        metadata.listener_bound = true;
        let task = tokio::spawn(async move {
            let mut bytes = [0; 512];
            let (size, peer) = socket.recv_from(&mut bytes).await.unwrap();
            let request = Message::from_vec(&bytes[..size]).unwrap();
            let mut reply = Message::response(request.metadata.id, OpCode::Query);
            reply.queries = request.queries;
            match behavior {
                0 => reply.metadata.id = reply.metadata.id.wrapping_add(1),
                1 => {
                    reply.queries[0] =
                        Query::query(Name::from_ascii("wrong.test.").unwrap(), RecordType::A)
                }
                2 => {
                    socket.send_to(&[0; 513], peer).await.unwrap();
                    return;
                }
                3 => {
                    let other = UdpSocket::bind("127.0.0.1:0").await.unwrap();
                    other.send_to(&reply.to_vec().unwrap(), peer).await.unwrap();
                    return;
                }
                _ => {
                    tokio::time::sleep(Duration::from_secs(4)).await;
                    return;
                }
            }
            socket
                .send_to(&reply.to_vec().unwrap(), peer)
                .await
                .unwrap();
        });
        let (name, kind) = question("node.meshway.test", None).unwrap();
        assert!(matches!(
            query_listener(metadata, name, kind, false).await,
            Err(Error::InvalidResponse | Error::Timeout)
        ));
        task.abort();
        let _ = task.await;
    }
}

#[tokio::test]
async fn malicious_tcp_framing_oversize_partial_extra_and_timeout_are_failures() {
    for behavior in [0, 1, 2, 3] {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let endpoint = listener.local_addr().unwrap();
        let mut metadata = Status::unavailable();
        metadata.listener = Some(endpoint);
        metadata.listener_bound = true;
        let task = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let length = socket.read_u16().await.unwrap();
            let mut request = vec![0; length as usize];
            socket.read_exact(&mut request).await.unwrap();
            match behavior {
                0 => socket.write_u16(4097).await.unwrap(),
                1 => {
                    socket.write_u16(100).await.unwrap();
                    socket.write_all(&[0; 12]).await.unwrap();
                }
                2 => {
                    let request = Message::from_vec(&request).unwrap();
                    let mut reply = Message::response(request.metadata.id, OpCode::Query);
                    reply.queries = request.queries;
                    let bytes = reply.to_vec().unwrap();
                    socket.write_u16(bytes.len() as u16).await.unwrap();
                    socket.write_all(&bytes).await.unwrap();
                    socket.write_all(&[0]).await.unwrap();
                }
                _ => tokio::time::sleep(Duration::from_secs(4)).await,
            }
        });
        let (name, kind) = question("node.meshway.test", None).unwrap();
        assert!(matches!(
            query_listener(metadata, name, kind, true).await,
            Err(Error::InvalidResponse | Error::Timeout)
        ));
        task.abort();
        let _ = task.await;
    }
}
