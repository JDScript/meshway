use crate::{Query, WireError, decode_query, wire::MIN_MESSAGE_SIZE};
use bytes::{Bytes, BytesMut};
use std::io;
use tokio_util::codec::{Decoder, Encoder, LengthDelimitedCodec};

#[derive(Debug, thiserror::Error)]
pub enum TcpError {
    #[error("DNS TCP framing failed: {0}")]
    Framing(#[from] io::Error),
    #[error(transparent)]
    Wire(#[from] WireError),
}

/// Incremental DNS-over-TCP query decoding and response framing.
///
/// Delegates big-endian u16 framing and maximum-frame enforcement to Tokio's
/// length-delimited codec. The caller owns aggregate buffering/backpressure.
/// Discard the stream after a framing or query error. Encoder input is one DNS
/// payload from [`crate::encode_response`], without a prefix.
#[derive(Debug)]
pub struct TcpCodec {
    framing: LengthDelimitedCodec,
    incomplete: bool,
}

impl TcpCodec {
    pub fn new(max_message_size: u16) -> Result<Self, WireError> {
        if usize::from(max_message_size) < MIN_MESSAGE_SIZE {
            return Err(WireError::MessageLength);
        }
        Ok(Self {
            framing: LengthDelimitedCodec::builder()
                .big_endian()
                .length_field_type::<u16>()
                .max_frame_length(usize::from(max_message_size))
                .new_codec(),
            incomplete: false,
        })
    }
}

impl Decoder for TcpCodec {
    type Item = Query;
    type Error = TcpError;

    fn decode(&mut self, src: &mut BytesMut) -> Result<Option<Query>, TcpError> {
        let had_bytes = !src.is_empty();
        let frame = self.framing.decode(src)?;
        self.incomplete = frame.is_none() && (self.incomplete || had_bytes);
        frame
            .map(|frame| decode_query(&frame).map_err(TcpError::from))
            .transpose()
    }

    fn decode_eof(&mut self, src: &mut BytesMut) -> Result<Option<Query>, TcpError> {
        let frame = self.decode(src)?;
        // LengthDelimitedCodec can consume the prefix while awaiting the body.
        // An empty buffer at EOF therefore need not mean a complete stream.
        if frame.is_none() && self.incomplete {
            return Err(
                io::Error::new(io::ErrorKind::UnexpectedEof, "incomplete DNS frame").into(),
            );
        }
        Ok(frame)
    }
}

impl Encoder<Bytes> for TcpCodec {
    type Error = TcpError;

    fn encode(&mut self, payload: Bytes, dst: &mut BytesMut) -> Result<(), TcpError> {
        if payload.len() < MIN_MESSAGE_SIZE {
            return Err(WireError::MessageLength.into());
        }
        self.framing.encode(payload, dst)?;
        Ok(())
    }
}
