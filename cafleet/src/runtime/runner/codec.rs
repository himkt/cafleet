use super::*;

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
    ensure(
        body.len() <= MAX_CONTROL_FRAME,
        "control frame exceeds size limit",
    )?;
    let mut bytes = u32::try_from(body.len())
        .map_err(io_error)?
        .to_be_bytes()
        .to_vec();
    bytes.extend(body);
    Ok(bytes)
}

pub(crate) struct ControlDecoder {
    header: [u8; 4],
    header_len: usize,
    body: Vec<u8>,
    expected: Option<usize>,
}

impl ControlDecoder {
    pub(crate) fn new() -> Self {
        Self {
            header: [0; 4],
            header_len: 0,
            body: Vec::new(),
            expected: None,
        }
    }

    pub(crate) fn feed(&mut self, mut bytes: &[u8]) -> Result<Vec<Value>> {
        let mut frames = Vec::new();
        while !bytes.is_empty() {
            if self.expected.is_none() {
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
                let frame = serde_json::from_slice(&self.body)
                    .map_err(|_| error("malformed control JSON body"))?;
                validate_version(&frame)?;
                frames.push(frame);
                self.body.clear();
                self.header_len = 0;
                self.expected = None;
            }
        }
        Ok(frames)
    }

    pub(crate) fn finish(&self) -> Result<()> {
        ensure(
            self.header_len == 0 && self.expected.is_none(),
            "truncated control frame",
        )
    }
}
