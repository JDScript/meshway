use std::{
    fmt, io,
    pin::Pin,
    task::{Context, Poll, ready},
};

use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};

use crate::{ClientConfig, Error, check_addons, check_version, write_client_request};

/// Client-side stream with lazy, bounded response-header validation on first read.
///
/// Writes are available immediately, so an external VLESS proxy can forward a
/// client-first Noise handshake to its destination before returning its response.
/// Reads never expose VLESS headers as Meshway payload. Once validation fails,
/// subsequent reads and writes fail; the caller must close the connection.
/// The two response bytes are the only buffered data. No payload is buffered.
///
/// Construct with `client_stream`; `S` may itself be a borrowed or TLS stream.
pub struct ClientStream<S> {
    inner: S,
    response: [u8; 2],
    response_len: usize,
    response_validated: bool,
    failed: bool,
}

impl<S> fmt::Debug for ClientStream<S> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ClientStream")
            .field("response_validated", &self.response_validated)
            .field("failed", &self.failed)
            .finish_non_exhaustive()
    }
}

/// Write and flush the request, returning a stream which consumes and validates
/// the server response on first read. Successful construction does not mean the
/// proxy accepted the request; run Noise with a deadline over this stream.
pub async fn client_stream<S>(
    mut stream: S,
    config: &ClientConfig,
) -> Result<ClientStream<S>, Error>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    write_client_request(&mut stream, config).await?;
    Ok(ClientStream {
        inner: stream,
        response: [0; 2],
        response_len: 0,
        response_validated: false,
        failed: false,
    })
}

impl<S> ClientStream<S> {
    /// Only confirms the VLESS response header, not Meshway peer authentication.
    pub fn response_validated(&self) -> bool {
        self.response_validated
    }

    /// Recover the original stream only after consuming a valid response header.
    /// On failure this returns the adapter without discarding its parsing state.
    pub fn try_into_inner(self) -> Result<S, Self> {
        if self.response_validated && !self.failed {
            Ok(self.inner)
        } else {
            Err(self)
        }
    }
}

impl<S: AsyncRead + Unpin> AsyncRead for ClientStream<S> {
    fn poll_read(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buffer: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        let this = self.get_mut();
        if this.failed {
            return Poll::Ready(Err(failed_handshake()));
        }
        if buffer.remaining() == 0 {
            return Poll::Ready(Ok(()));
        }
        while !this.response_validated {
            // One fixed field at a time: unknown versions fail without waiting
            // for a second byte, and cancellation preserves the parsing state.
            let offset = this.response_len;
            let mut header = ReadBuf::new(&mut this.response[offset..offset + 1]);
            match ready!(Pin::new(&mut this.inner).poll_read(cx, &mut header)) {
                Ok(()) if !header.filled().is_empty() => {}
                Ok(()) => {
                    this.failed = true;
                    return Poll::Ready(Err(io::Error::new(
                        io::ErrorKind::UnexpectedEof,
                        "truncated VLESS response",
                    )));
                }
                Err(error) => {
                    this.failed = true;
                    return Poll::Ready(Err(error));
                }
            }
            this.response_len += 1;
            let validation = if offset == 0 {
                check_version(this.response[0])
            } else {
                check_addons(this.response[1])
            };
            if let Err(error) = validation {
                this.failed = true;
                return Poll::Ready(Err(error.into_io()));
            }
            this.response_validated = this.response_len == 2;
        }
        Pin::new(&mut this.inner).poll_read(cx, buffer)
    }
}

impl<S: AsyncWrite + Unpin> AsyncWrite for ClientStream<S> {
    fn poll_write(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buffer: &[u8],
    ) -> Poll<io::Result<usize>> {
        let this = self.get_mut();
        if this.failed {
            return Poll::Ready(Err(failed_handshake()));
        }
        Pin::new(&mut this.inner).poll_write(cx, buffer)
    }

    fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        let this = self.get_mut();
        if this.failed {
            return Poll::Ready(Err(failed_handshake()));
        }
        Pin::new(&mut this.inner).poll_flush(cx)
    }

    fn poll_shutdown(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        // Shutdown remains available even after rejection.
        Pin::new(&mut self.get_mut().inner).poll_shutdown(cx)
    }
}

fn failed_handshake() -> io::Error {
    io::Error::new(
        io::ErrorKind::InvalidData,
        "VLESS response validation failed",
    )
}
