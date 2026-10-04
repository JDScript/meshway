use thiserror::Error;

/// Validated per-session send budgets. Queued and in-flight operations share
/// these budgets. Bytes charge **Vec capacity**, not just payload length.
///
/// In addition to admitted payloads, a stream reserves `stream_buffer_bytes()`
/// for its encoded write group and temporarily copies at most one
/// `max_frame_bytes` payload into the encoder. Its read codec starts with
/// `min(stream_buffer_bytes(), 64 KiB)` read-ahead capacity and can grow for a
/// validated maximum-size frame. Including BytesMut's growth/reclamation slack,
/// its backing allocation is bounded by four times the larger of that initial
/// capacity and `max_frame_bytes + 4`. A datagram endpoint instead owns one
/// `max_frame_bytes` receive buffer. Raw-provider/OS buffering is separate.
///
/// Metadata is O(max_frames + max_operations). Returned completions and
/// received payload Vecs belong to the caller; retaining them requires a
/// separate host budget. There is no receive worker or unbounded completion
/// queue. These scratch bounds are additional to the admitted Vec-capacity
/// budget; encoding does not acquire another admission permit.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Limits {
    max_frame_bytes: usize,
    max_frames: usize,
    max_bytes: usize,
    max_operations: usize,
    max_batch_frames: usize,
    stream_buffer_bytes: usize,
}

#[derive(Clone, Copy, Debug, Eq, Error, PartialEq)]
#[error("invalid transport budget: {0}")]
pub struct LimitError(pub &'static str);

impl Limits {
    /// Engineering caps: 16 MiB/frame, 1 GiB admitted payload capacity,
    /// 65,536 admitted frames/operations. All values must be nonzero; one
    /// maximum-size frame must fit, and a batch must fit the frame budget.
    pub fn new(
        max_frame_bytes: usize,
        max_frames: usize,
        max_bytes: usize,
        max_operations: usize,
        max_batch_frames: usize,
    ) -> Result<Self, LimitError> {
        if !(1..=16 * 1024 * 1024).contains(&max_frame_bytes) {
            return Err(LimitError("max_frame_bytes must be in 1..=16 MiB"));
        }
        if !(1..=65_536).contains(&max_frames)
            || !(1..=65_536).contains(&max_operations)
            || max_operations > max_frames
        {
            return Err(LimitError("operations <= frames <= 65536, both nonzero"));
        }
        if max_bytes < max_frame_bytes || max_bytes > 1024 * 1024 * 1024 {
            return Err(LimitError("max_frame_bytes <= max_bytes <= 1 GiB"));
        }
        if max_batch_frames == 0 || max_batch_frames > max_frames {
            return Err(LimitError("batch frames must be in 1..=max_frames"));
        }
        // Also keep all semaphore and wire-length conversions checked on
        // every supported pointer width, rather than relying on defaults.
        for count in [max_frame_bytes, max_frames, max_bytes, max_operations] {
            if u32::try_from(count).is_err() || count > tokio::sync::Semaphore::MAX_PERMITS {
                return Err(LimitError("budget is not representable"));
            }
        }
        let maximum_wire_frame = max_frame_bytes
            .checked_add(4)
            .ok_or(LimitError("framed size overflow"))?;
        let maximum_wire_batch = max_batch_frames
            .checked_mul(4)
            .and_then(|headers| max_bytes.checked_add(headers))
            .ok_or(LimitError("batch framing budget overflow"))?;
        let stream_buffer_bytes = maximum_wire_frame.max(maximum_wire_batch.min(256 * 1024));
        Ok(Self {
            max_frame_bytes,
            max_frames,
            max_bytes,
            max_operations,
            max_batch_frames,
            stream_buffer_bytes,
        })
    }

    pub fn max_frame_bytes(self) -> usize {
        self.max_frame_bytes
    }
    pub fn max_frames(self) -> usize {
        self.max_frames
    }
    pub fn max_bytes(self) -> usize {
        self.max_bytes
    }
    pub fn max_operations(self) -> usize {
        self.max_operations
    }
    pub fn max_batch_frames(self) -> usize {
        self.max_batch_frames
    }

    /// Maximum encoded bytes in one stream write/flush group, including each
    /// u32 length prefix: `max(max_frame_bytes + 4,
    /// min(max_bytes + 4 * max_batch_frames, 256 KiB))`.
    ///
    /// A permitted single large frame therefore still fits above the 256 KiB
    /// grouping target. Groups never span operations/SendContexts and complete
    /// only after the raw writer's required flush. The stream read codec's
    /// initial capacity is `min(self.stream_buffer_bytes(), 64 * 1024)`;
    /// comparative raw readers can use that same BufReader capacity.
    pub fn stream_buffer_bytes(self) -> usize {
        self.stream_buffer_bytes
    }
}
