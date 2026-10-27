use super::*;
use std::time::Instant;

#[cfg(test)]
mod input_frames_tests;

pub(crate) const CONTROL_PROTOCOL_VERSION: u32 = 1;
pub(crate) const MAX_CONTROL_FRAME: usize = 65536;

fn validate_version(frame: &Value) -> Result<()> {
    ensure(
        frame.is_object()
            && frame.get("protocol_version").and_then(Value::as_u64)
                == Some(u64::from(CONTROL_PROTOCOL_VERSION)),
        "control frame requires integer protocol_version 1",
    )
}

pub(crate) fn encode_control_frame(frame: &Value) -> Result<Vec<u8>> {
    validate_version(frame)?;
    let body = serde_json::to_vec(frame).map_err(io_error)?;
    frame_control_body(&body)
}

pub(super) fn frame_control_body(body: &[u8]) -> Result<Vec<u8>> {
    ensure(
        body.len() <= MAX_CONTROL_FRAME,
        "control frame exceeds size limit",
    )?;
    let mut bytes = u32::try_from(body.len())
        .map_err(io_error)?
        .to_be_bytes()
        .to_vec();
    bytes.extend_from_slice(body);
    Ok(bytes)
}

pub(super) fn decode_legacy_control_body(body: &[u8]) -> Result<Value> {
    let frame = serde_json::from_slice(body).map_err(|_| error("malformed control JSON body"))?;
    validate_version(&frame)?;
    Ok(frame)
}

pub(super) struct AdmittedControlFrame {
    body: Vec<u8>,
    admitted_at: Instant,
}
impl AdmittedControlFrame {
    pub(super) fn body(&self) -> &[u8] {
        &self.body
    }
    pub(super) fn admitted_at(&self) -> Instant {
        self.admitted_at
    }
}

pub(crate) struct ControlDecoder {
    header: [u8; 4],
    header_len: usize,
    body: Vec<u8>,
    expected: Option<usize>,
    admitted_at: Option<Instant>,
}

impl ControlDecoder {
    pub(crate) fn new() -> Self {
        Self {
            header: [0; 4],
            header_len: 0,
            body: Vec::new(),
            expected: None,
            admitted_at: None,
        }
    }

    #[cfg_attr(
        not(test),
        expect(
            dead_code,
            reason = "the compatible Value decoder remains available beside the shared admitted framer"
        )
    )]
    pub(crate) fn feed(&mut self, bytes: &[u8]) -> Result<Vec<Value>> {
        self.feed_admitted(bytes, Instant::now())?
            .into_iter()
            .map(|frame| decode_legacy_control_body(frame.body()))
            .collect()
    }

    pub(super) fn feed_admitted(
        &mut self,
        mut bytes: &[u8],
        observed_at: Instant,
    ) -> Result<Vec<AdmittedControlFrame>> {
        let mut frames = Vec::new();
        while !bytes.is_empty() {
            if self.expected.is_none() {
                if self.header_len == 0 {
                    self.admitted_at = Some(observed_at);
                }
                let count = (4 - self.header_len).min(bytes.len());
                self.header[self.header_len..self.header_len + count]
                    .copy_from_slice(&bytes[..count]);
                self.header_len += count;
                bytes = &bytes[count..];
                if self.header_len < 4 {
                    continue;
                }
                let length = usize::try_from(u32::from_be_bytes(self.header)).map_err(io_error)?;
                ensure(
                    length <= MAX_CONTROL_FRAME,
                    "control frame exceeds size limit",
                )?;
                self.expected = Some(length);
            }
            let length = self
                .expected
                .expect("a complete header declares the body length");
            let count = (length - self.body.len()).min(bytes.len());
            self.body.extend_from_slice(&bytes[..count]);
            bytes = &bytes[count..];
            if self.body.len() == length {
                frames.push(AdmittedControlFrame {
                    body: std::mem::take(&mut self.body),
                    admitted_at: self
                        .admitted_at
                        .take()
                        .expect("a complete frame consumed its first header byte"),
                });
                self.header_len = 0;
                self.expected = None;
            }
        }
        Ok(frames)
    }

    pub(crate) fn finish(&self) -> Result<()> {
        ensure(
            self.header_len == 0 && self.expected.is_none() && self.admitted_at.is_none(),
            "truncated control frame",
        )
    }
}
