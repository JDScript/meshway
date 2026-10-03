//! Bounded SSE framing. Receiving an event never acknowledges its application.
mod decoder;

use bytes::{Buf, Bytes};
use decoder::Decoder;
use serde::de::DeserializeOwned;
use std::{marker::PhantomData, time::Duration};

use crate::ClientError;

#[derive(Default)]
pub struct StreamOptions<'a> {
    /// Last cursor whose complete state was applied by the caller, not merely decoded.
    pub applied_cursor: Option<&'a str>,
    /// Per-line and per-event UTF-8 budget; defaults to the client's body budget.
    pub max_frame_bytes: Option<usize>,
}

/// HTTP status contract supplied by generated operations.
pub struct StreamResponse<'a, P> {
    pub(crate) success_status: u16,
    pub(crate) error_statuses: &'a [u16],
    pub(crate) problem: Option<ProblemDecoder<P>>,
}
type ProblemDecoder<P> = fn(&[u8]) -> Option<P>;
impl<'a, P: DeserializeOwned> StreamResponse<'a, P> {
    pub fn json(success_status: u16, error_statuses: &'a [u16]) -> Self {
        Self {
            success_status,
            error_statuses,
            problem: Some(|bytes| serde_json::from_slice(bytes).ok()),
        }
    }
}
impl StreamResponse<'_, std::convert::Infallible> {
    pub fn untyped(success_status: u16) -> Self {
        Self {
            success_status,
            error_statuses: &[],
            problem: None,
        }
    }
}

pub struct EventDelivery<T> {
    pub data: T,
    /// A frame-local suggestion only. Snapshot frames may have no id.
    pub id: Option<String>,
}

/// One SSE connection with no background task or implicit reconnect. Drop the
/// stream to cancel; dropping only `next()` preserves the connection and decoder.
pub struct EventStream<T, P> {
    response: Option<reqwest::Response>,
    buffered: Bytes,
    decoder: Decoder,
    marker: PhantomData<fn() -> (T, P)>,
}

impl<T: DeserializeOwned, P> EventStream<T, P> {
    pub(crate) fn new(response: reqwest::Response, limit: usize) -> Self {
        Self {
            response: Some(response),
            buffered: Bytes::new(),
            decoder: Decoder::new(limit),
            marker: PhantomData,
        }
    }

    /// Bounded server retry hint. The caller chooses its retry policy and lease.
    pub fn retry_delay(&self) -> Option<Duration> {
        self.decoder.retry_delay()
    }

    /// EOF discards partial frames. Decode/transport errors are terminal and
    /// release the response before being returned; all later reads return None.
    pub async fn next(&mut self) -> Option<Result<EventDelivery<T>, ClientError<P>>> {
        loop {
            let response = self.response.as_mut()?;
            let status = response.status().as_u16();
            while self.buffered.has_remaining() {
                match self.decoder.push(self.buffered.get_u8()) {
                    Ok(Some(frame)) => {
                        let value = if frame.event == "message" {
                            serde_json::from_str(&frame.data)
                                .ok()
                                .map(|data| EventDelivery { data, id: frame.id })
                        } else {
                            None
                        };
                        if let Some(value) = value {
                            return Some(Ok(value));
                        }
                        self.close();
                        return Some(Err(ClientError::Decode { status }));
                    }
                    Ok(None) => {}
                    Err(error) => {
                        self.close();
                        return Some(Err(match error {
                            decoder::Error::Decode => ClientError::Decode { status },
                            decoder::Error::Limit => ClientError::BodyLimit { status },
                        }));
                    }
                }
            }
            match response.chunk().await {
                Ok(Some(chunk)) => self.buffered = chunk,
                Ok(None) => {
                    self.close();
                    return None;
                }
                Err(_) => {
                    self.close();
                    return Some(Err(ClientError::Network {
                        status: Some(status),
                    }));
                }
            }
        }
    }

    fn close(&mut self) {
        self.response = None;
        self.buffered = Bytes::new();
        self.decoder.clear();
    }
}

#[cfg(test)]
mod tests;
