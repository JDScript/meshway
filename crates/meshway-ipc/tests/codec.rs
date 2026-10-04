use meshway_ipc::{CodecError, Request};
use tokio::io::AsyncWriteExt;

async fn decode(bytes: &[u8]) -> Result<Request, CodecError> {
    let (mut write, mut read) = tokio::io::duplex(128 * 1024);
    write.write_u32(bytes.len() as u32).await.unwrap();
    write.write_all(bytes).await.unwrap();
    drop(write);
    meshway_ipc::read(&mut read).await
}
#[tokio::test]
async fn local_messages_reject_duplicate_unknown_positional_and_oversize_inputs() {
    let valid = br#"{"version":1,"requestId":"00000000-0000-4000-8000-000000000001","command":{"type":"status"}}"#;
    assert!(decode(valid).await.is_ok());
    for invalid in [
        br#"{"version":1,"version":1,"requestId":"00000000-0000-4000-8000-000000000001","command":{"type":"status"}}"#.as_slice(),
        br#"{"version":1,"requestId":"00000000-0000-4000-8000-000000000001","command":{"type":"status","uid":0}}"#.as_slice(),
        br#"[1,"00000000-0000-4000-8000-000000000001",{"type":"status"}]"#.as_slice(),
    ] { assert!(decode(invalid).await.is_err()); }
    let (mut write, mut read) = tokio::io::duplex(32);
    write
        .write_u32((meshway_ipc::MAX_MESSAGE_BYTES + 1) as u32)
        .await
        .unwrap();
    assert!(matches!(
        meshway_ipc::read::<Request>(&mut read).await,
        Err(CodecError::Limit)
    ));
    let (mut write, mut read) = tokio::io::duplex(32);
    write.write_u32(20).await.unwrap();
    write.write_all(b"{").await.unwrap();
    drop(write);
    assert!(matches!(
        meshway_ipc::read::<Request>(&mut read).await,
        Err(CodecError::Io)
    ));
}
