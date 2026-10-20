//! Tagged, bounded machine streams multiplexed alongside the legacy proxy frames.
use uuid::Uuid;
const MAGIC: &[u8; 4] = b"NYXM";
pub const HEADER_BYTES: usize = 30;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum Kind {
    Desktop = 1,
    Input = 2,
    GatewayUpload = 3,
    GatewayDownload = 4,
    File = 5,
    GatewayUploadAbort = 6,
    GatewayDownloadAbort = 7,
    ProxyUpload = 8,
    ProxyUploadAbort = 9,
    GatewayCancel = 10,
    DesktopActivity = 11,
}
pub struct Frame<'a> {
    pub kind: Kind,
    pub end: bool,
    pub id: Uuid,
    pub sequence: u64,
    pub bytes: &'a [u8],
}
impl std::fmt::Debug for Frame<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Frame")
            .field("kind", &self.kind)
            .field("end", &self.end)
            .field("id", &self.id)
            .field("sequence", &self.sequence)
            .field("bytes", &"[redacted]")
            .finish()
    }
}
impl Frame<'_> {
    pub fn encode(&self) -> Result<Vec<u8>, &'static str> {
        let limit = if self.kind == Kind::Desktop {
            crate::MAX_FRAME_BYTES
        } else {
            crate::STREAM_CHUNK_BYTES
        };
        if self.bytes.len() > limit {
            return Err("machine frame size limit exceeded");
        }
        let mut frame = Vec::with_capacity(HEADER_BYTES + self.bytes.len());
        frame.extend_from_slice(MAGIC);
        frame.push(self.kind as u8);
        frame.push(u8::from(self.end));
        frame.extend_from_slice(self.id.as_bytes());
        frame.extend_from_slice(&self.sequence.to_be_bytes());
        frame.extend_from_slice(self.bytes);
        Ok(frame)
    }
    pub fn decode(bytes: &[u8]) -> Result<Frame<'_>, &'static str> {
        if bytes.len() < HEADER_BYTES || !bytes.starts_with(MAGIC) || bytes[5] > 1 {
            return Err("invalid machine frame");
        }
        let kind = match bytes[4] {
            1 => Kind::Desktop,
            2 => Kind::Input,
            3 => Kind::GatewayUpload,
            4 => Kind::GatewayDownload,
            5 => Kind::File,
            6 => Kind::GatewayUploadAbort,
            7 => Kind::GatewayDownloadAbort,
            8 => Kind::ProxyUpload,
            9 => Kind::ProxyUploadAbort,
            10 => Kind::GatewayCancel,
            11 => Kind::DesktopActivity,
            _ => return Err("unknown machine frame kind"),
        };
        let limit = if kind == Kind::Desktop {
            crate::MAX_FRAME_BYTES
        } else {
            crate::STREAM_CHUNK_BYTES
        };
        if bytes.len() - HEADER_BYTES > limit {
            return Err("machine frame size limit exceeded");
        }
        Ok(Frame {
            kind,
            end: bytes[5] == 1,
            id: Uuid::from_slice(&bytes[6..22]).map_err(|_| "invalid frame ID")?,
            sequence: u64::from_be_bytes(bytes[22..30].try_into().map_err(|_| "invalid sequence")?),
            bytes: &bytes[30..],
        })
    }
}
pub fn is_machine(bytes: &[u8]) -> bool {
    bytes.starts_with(MAGIC)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn bounded_tagged_round_trip() {
        let id = Uuid::new_v4();
        let raw = Frame {
            kind: Kind::GatewayUpload,
            end: true,
            id,
            sequence: 19,
            bytes: b"payload",
        }
        .encode()
        .unwrap();
        let parsed = Frame::decode(&raw).unwrap();
        assert_eq!(parsed.id, id);
        assert_eq!(parsed.sequence, 19);
        assert!(parsed.end);
        assert_eq!(parsed.bytes, b"payload");
        assert!(!is_machine(id.to_string().as_bytes()));
        assert!(Frame::decode(b"NYXM").is_err());
        assert!(
            Frame {
                kind: Kind::Input,
                end: false,
                id,
                sequence: 0,
                bytes: &vec![0; crate::STREAM_CHUNK_BYTES + 1]
            }
            .encode()
            .is_err()
        );
    }
}
