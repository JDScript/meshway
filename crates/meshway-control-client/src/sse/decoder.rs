use std::time::Duration;

pub(super) enum Error {
    Decode,
    Limit,
}
pub(super) struct Frame {
    pub data: String,
    pub event: String,
    pub id: Option<String>,
}

pub(super) struct Decoder {
    line: Vec<u8>,
    limit: usize,
    data: String,
    has_data: bool,
    size: usize,
    event: String,
    id: Option<String>,
    skip_lf: bool,
    first_line: bool,
    retry: Option<Duration>,
}

impl Decoder {
    pub(super) fn new(limit: usize) -> Self {
        Self {
            line: Vec::new(),
            limit,
            data: String::new(),
            has_data: false,
            size: 0,
            event: String::new(),
            id: None,
            skip_lf: false,
            first_line: true,
            retry: None,
        }
    }
    pub(super) fn retry_delay(&self) -> Option<Duration> {
        self.retry
    }
    pub(super) fn clear(&mut self) {
        self.line.clear();
        self.data.clear();
        self.id = None;
        self.event.clear();
        self.size = 0;
        self.has_data = false;
    }
    pub(super) fn push(&mut self, byte: u8) -> Result<Option<Frame>, Error> {
        if self.skip_lf {
            self.skip_lf = false;
            if byte == b'\n' {
                return Ok(None);
            }
        }
        if byte != b'\r' && byte != b'\n' {
            if self.line.len() >= self.limit {
                return Err(Error::Limit);
            }
            self.line.push(byte);
            return Ok(None);
        }
        self.skip_lf = byte == b'\r';
        let line = std::mem::take(&mut self.line);
        let mut text = std::str::from_utf8(&line).map_err(|_| Error::Decode)?;
        if self.first_line {
            self.first_line = false;
            text = text.strip_prefix('\u{feff}').unwrap_or(text);
        }
        if text.is_empty() {
            let frame = self.has_data.then(|| {
                self.data.pop();
                Frame {
                    data: std::mem::take(&mut self.data),
                    event: if self.event.is_empty() {
                        "message".into()
                    } else {
                        std::mem::take(&mut self.event)
                    },
                    id: self.id.take(),
                }
            });
            self.clear();
            return Ok(frame);
        }
        if text.starts_with(':') {
            return Ok(None);
        }
        let (name, value) = text.split_once(':').unwrap_or((text, ""));
        let value = value.strip_prefix(' ').unwrap_or(value);
        match name {
            "retry" if !value.is_empty() && value.bytes().all(|b| b.is_ascii_digit()) => {
                if let Ok(ms) = value.parse::<u64>() {
                    // Same JS-safe hint boundary as the TypeScript decoder.
                    if ms <= 9_007_199_254_740_991 {
                        self.retry = Some(Duration::from_millis(ms.clamp(100, 60_000)));
                    }
                }
            }
            "data" => {
                self.size += value.len() + 1;
                self.data.push_str(value);
                self.data.push('\n');
                self.has_data = true;
            }
            "id" if !value.contains('\0') => {
                self.size += value.len();
                self.id = Some(value.into());
            }
            "event" => {
                self.size += value.len();
                self.event = value.into();
            }
            _ => {}
        }
        if self.size > self.limit {
            return Err(Error::Limit);
        }
        Ok(None)
    }
}
