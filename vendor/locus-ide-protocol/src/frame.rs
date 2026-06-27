use serde::{de::DeserializeOwned, Deserialize, Serialize};

use crate::ids::{CorrelationId, Seq};
use crate::message::{HostToWorker, MessageValidationError, WorkerToHost};
use crate::version::{PROTOCOL_MAJOR, PROTOCOL_MINOR};

pub const MAX_FRAME_PAYLOAD_BYTES: u32 = 1024 * 1024;
pub const FRAME_HEADER_BYTES: usize = 28;

#[repr(u16)]
#[derive(Clone, Copy, Debug, Eq, PartialEq, Deserialize, Serialize)]
pub enum FrameKind {
    HostToWorker = 1,
    WorkerToHost = 2,
}

impl FrameKind {
    fn from_u16(value: u16) -> Option<Self> {
        match value {
            1 => Some(Self::HostToWorker),
            2 => Some(Self::WorkerToHost),
            _ => None,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct FrameHeader {
    pub major: u16,
    pub minor: u16,
    pub frame_kind: FrameKind,
    pub flags: u16,
    pub payload_len: u32,
    pub seq: Seq,
    pub correlation_id: CorrelationId,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FrameDecode<'a> {
    pub header: FrameHeader,
    pub payload: &'a [u8],
}

#[derive(Clone, Debug, PartialEq)]
pub enum FrameError {
    TruncatedHeader {
        actual: usize,
    },
    TruncatedPayload {
        expected: usize,
        actual: usize,
    },
    TrailingBytes {
        expected: usize,
        actual: usize,
    },
    UnsupportedMajor {
        found: u16,
        expected: u16,
    },
    UnsupportedMinor {
        found: u16,
        max: u16,
    },
    UnknownFrameKind(u16),
    PayloadTooLarge {
        len: u32,
        max: u32,
    },
    Encode(String),
    Decode(String),
    InvalidHostMessage(MessageValidationError),
    InvalidWorkerMessage(MessageValidationError),
    CorrelationMismatch {
        header: CorrelationId,
        message: CorrelationId,
    },
    UnexpectedFrameKind {
        found: FrameKind,
        expected: FrameKind,
    },
}

pub fn encode_host_to_worker(
    seq: Seq,
    correlation_id: CorrelationId,
    message: &HostToWorker,
) -> Result<Vec<u8>, FrameError> {
    message.validate().map_err(FrameError::InvalidHostMessage)?;
    validate_message_correlation(correlation_id, host_message_correlation(message))?;
    encode_frame(FrameKind::HostToWorker, seq, correlation_id, message)
}

pub fn encode_worker_to_host(
    seq: Seq,
    correlation_id: CorrelationId,
    message: &WorkerToHost,
) -> Result<Vec<u8>, FrameError> {
    message
        .validate()
        .map_err(FrameError::InvalidWorkerMessage)?;
    validate_message_correlation(correlation_id, worker_message_correlation(message))?;
    encode_frame(FrameKind::WorkerToHost, seq, correlation_id, message)
}

pub fn decode_host_to_worker(bytes: &[u8]) -> Result<(FrameHeader, HostToWorker), FrameError> {
    let decoded = decode_frame(bytes)?;
    if decoded.header.frame_kind != FrameKind::HostToWorker {
        return Err(FrameError::UnexpectedFrameKind {
            found: decoded.header.frame_kind,
            expected: FrameKind::HostToWorker,
        });
    }
    let msg: HostToWorker =
        postcard::from_bytes(decoded.payload).map_err(|e| FrameError::Decode(e.to_string()))?;
    msg.validate().map_err(FrameError::InvalidHostMessage)?;
    validate_message_correlation(
        decoded.header.correlation_id,
        host_message_correlation(&msg),
    )?;
    Ok((decoded.header, msg))
}

pub fn decode_worker_to_host(bytes: &[u8]) -> Result<(FrameHeader, WorkerToHost), FrameError> {
    let decoded = decode_frame(bytes)?;
    if decoded.header.frame_kind != FrameKind::WorkerToHost {
        return Err(FrameError::UnexpectedFrameKind {
            found: decoded.header.frame_kind,
            expected: FrameKind::WorkerToHost,
        });
    }
    let msg: WorkerToHost =
        postcard::from_bytes(decoded.payload).map_err(|e| FrameError::Decode(e.to_string()))?;
    msg.validate().map_err(FrameError::InvalidWorkerMessage)?;
    validate_message_correlation(
        decoded.header.correlation_id,
        worker_message_correlation(&msg),
    )?;
    Ok((decoded.header, msg))
}

pub fn encode_frame<T: Serialize>(
    frame_kind: FrameKind,
    seq: Seq,
    correlation_id: CorrelationId,
    payload: &T,
) -> Result<Vec<u8>, FrameError> {
    let payload = postcard::to_allocvec(payload).map_err(|e| FrameError::Encode(e.to_string()))?;
    if payload.len() > MAX_FRAME_PAYLOAD_BYTES as usize {
        return Err(FrameError::PayloadTooLarge {
            len: payload.len() as u32,
            max: MAX_FRAME_PAYLOAD_BYTES,
        });
    }
    let header = FrameHeader {
        major: PROTOCOL_MAJOR,
        minor: PROTOCOL_MINOR,
        frame_kind,
        flags: 0,
        payload_len: payload.len() as u32,
        seq,
        correlation_id,
    };
    let mut out = Vec::with_capacity(FRAME_HEADER_BYTES + payload.len());
    write_header(&mut out, header);
    out.extend_from_slice(&payload);
    Ok(out)
}

pub fn decode_frame(bytes: &[u8]) -> Result<FrameDecode<'_>, FrameError> {
    if bytes.len() < FRAME_HEADER_BYTES {
        return Err(FrameError::TruncatedHeader {
            actual: bytes.len(),
        });
    }
    let header = read_header(&bytes[..FRAME_HEADER_BYTES])?;
    if header.payload_len > MAX_FRAME_PAYLOAD_BYTES {
        return Err(FrameError::PayloadTooLarge {
            len: header.payload_len,
            max: MAX_FRAME_PAYLOAD_BYTES,
        });
    }
    let expected = FRAME_HEADER_BYTES + header.payload_len as usize;
    if bytes.len() < expected {
        return Err(FrameError::TruncatedPayload {
            expected,
            actual: bytes.len(),
        });
    }
    if bytes.len() > expected {
        return Err(FrameError::TrailingBytes {
            expected,
            actual: bytes.len(),
        });
    }
    Ok(FrameDecode {
        header,
        payload: &bytes[FRAME_HEADER_BYTES..expected],
    })
}

pub fn decode_payload<T: DeserializeOwned>(frame: &FrameDecode<'_>) -> Result<T, FrameError> {
    postcard::from_bytes(frame.payload).map_err(|e| FrameError::Decode(e.to_string()))
}

fn write_header(out: &mut Vec<u8>, header: FrameHeader) {
    out.extend_from_slice(&header.major.to_le_bytes());
    out.extend_from_slice(&header.minor.to_le_bytes());
    out.extend_from_slice(&(header.frame_kind as u16).to_le_bytes());
    out.extend_from_slice(&header.flags.to_le_bytes());
    out.extend_from_slice(&header.payload_len.to_le_bytes());
    out.extend_from_slice(&header.seq.get().to_le_bytes());
    out.extend_from_slice(&header.correlation_id.get().to_le_bytes());
}

fn read_header(bytes: &[u8]) -> Result<FrameHeader, FrameError> {
    let major = u16::from_le_bytes([bytes[0], bytes[1]]);
    let minor = u16::from_le_bytes([bytes[2], bytes[3]]);
    if major != PROTOCOL_MAJOR {
        return Err(FrameError::UnsupportedMajor {
            found: major,
            expected: PROTOCOL_MAJOR,
        });
    }
    if minor > PROTOCOL_MINOR {
        return Err(FrameError::UnsupportedMinor {
            found: minor,
            max: PROTOCOL_MINOR,
        });
    }
    let raw_kind = u16::from_le_bytes([bytes[4], bytes[5]]);
    let Some(frame_kind) = FrameKind::from_u16(raw_kind) else {
        return Err(FrameError::UnknownFrameKind(raw_kind));
    };
    let flags = u16::from_le_bytes([bytes[6], bytes[7]]);
    let payload_len = u32::from_le_bytes([bytes[8], bytes[9], bytes[10], bytes[11]]);
    let seq = Seq::new(u64::from_le_bytes([
        bytes[12], bytes[13], bytes[14], bytes[15], bytes[16], bytes[17], bytes[18], bytes[19],
    ]));
    let correlation_id = CorrelationId::new(u64::from_le_bytes([
        bytes[20], bytes[21], bytes[22], bytes[23], bytes[24], bytes[25], bytes[26], bytes[27],
    ]));
    Ok(FrameHeader {
        major,
        minor,
        frame_kind,
        flags,
        payload_len,
        seq,
        correlation_id,
    })
}

fn validate_message_correlation(
    header: CorrelationId,
    message: Option<CorrelationId>,
) -> Result<(), FrameError> {
    if let Some(message) = message {
        if header != message {
            return Err(FrameError::CorrelationMismatch { header, message });
        }
    }
    Ok(())
}

fn host_message_correlation(message: &HostToWorker) -> Option<CorrelationId> {
    match message {
        HostToWorker::PaneOpened { correlation_id, .. }
        | HostToWorker::PaneOpenFailed { correlation_id, .. }
        | HostToWorker::PaneCloseResult { correlation_id, .. }
        | HostToWorker::ConsoleInput { correlation_id, .. }
        | HostToWorker::PluginResult { correlation_id, .. }
        | HostToWorker::BulkAllocated { correlation_id, .. }
        | HostToWorker::BulkAllocationFailed { correlation_id, .. }
        | HostToWorker::BulkReleased { correlation_id, .. }
        | HostToWorker::PixelFramePresented { correlation_id, .. }
        | HostToWorker::DbTableCreated { correlation_id, .. }
        | HostToWorker::DbTableCreateFailed { correlation_id, .. }
        | HostToWorker::DbTableOpResult { correlation_id, .. }
        | HostToWorker::DbTableEventResult { correlation_id, .. }
        | HostToWorker::DpiResult { correlation_id, .. }
        | HostToWorker::SystemColorResult { correlation_id, .. }
        | HostToWorker::ShaderOpResult { correlation_id, .. } => Some(*correlation_id),
        _ => None,
    }
}

fn worker_message_correlation(message: &WorkerToHost) -> Option<CorrelationId> {
    match message {
        WorkerToHost::OpenPane { correlation_id, .. }
        | WorkerToHost::OpenShaderPane { correlation_id, .. }
        | WorkerToHost::ClosePane { correlation_id, .. }
        | WorkerToHost::ConsolePrompt { correlation_id, .. }
        | WorkerToHost::RequestPlugin { correlation_id, .. }
        | WorkerToHost::ConsoleReadRequest { correlation_id, .. }
        | WorkerToHost::AllocateBulk { correlation_id, .. }
        | WorkerToHost::ReleaseBulk { correlation_id, .. }
        | WorkerToHost::PresentPixels { correlation_id, .. }
        | WorkerToHost::SetShaderSource { correlation_id, .. }
        | WorkerToHost::SetShaderUniforms { correlation_id, .. }
        | WorkerToHost::SetShaderRedrawRate { correlation_id, .. }
        | WorkerToHost::SetShaderPlayback { correlation_id, .. }
        | WorkerToHost::DbTableCreate { correlation_id, .. }
        | WorkerToHost::DbTableAddColumn { correlation_id, .. }
        | WorkerToHost::DbTableShow { correlation_id, .. }
        | WorkerToHost::DbTablePollEvent { correlation_id, .. }
        | WorkerToHost::DbTableSetModel { correlation_id, .. }
        | WorkerToHost::DbTableSetCellText { correlation_id, .. }
        | WorkerToHost::DbTableSetPageData { correlation_id, .. }
        | WorkerToHost::DbTablePageComplete { correlation_id, .. }
        | WorkerToHost::DbTableSetError { correlation_id, .. }
        | WorkerToHost::DbTableClose { correlation_id, .. }
        | WorkerToHost::GetDpi { correlation_id, .. }
        | WorkerToHost::SystemColor { correlation_id, .. } => Some(*correlation_id),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bulk::{
        BulkAccess, BulkDescriptor, BulkKind, BulkTransport, BulkValidationError, MAX_BULK_BYTES,
    };
    use crate::draw::{
        Color, DrawBatch, DrawBatchValidationError, DrawCmd, PixelFormat, PixelFrame, Rect,
    };
    use crate::ids::{BulkId, PaneId, SessionId, TimerId};
    use crate::message::{SessionKind, StopReason};
    use crate::shader::{
        ShaderPlaybackState, ShaderSource, ShaderUniformUpdate, ShaderUniformValue,
    };
    use crate::trust::TrustLabel;

    #[test]
    fn host_to_worker_roundtrip() {
        let msg = HostToWorker::StartSession {
            session_id: SessionId::new(7),
            kind: SessionKind::Run,
            source: "42".to_string(),
        };
        let bytes = encode_host_to_worker(Seq::new(1), CorrelationId::new(99), &msg).unwrap();
        let (header, decoded) = decode_host_to_worker(&bytes).unwrap();
        assert_eq!(header.seq, Seq::new(1));
        assert_eq!(header.correlation_id, CorrelationId::new(99));
        assert_eq!(decoded, msg);
    }

    #[test]
    fn worker_to_host_pixel_frame_roundtrip_uses_bulk_descriptor() {
        let bulk = BulkDescriptor {
            bulk_id: BulkId::new(12),
            byte_len: 640 * 480 * 4,
            kind: BulkKind::PresentPixels,
            access: BulkAccess::ReadOnly,
            generation: 3,
            trust: TrustLabel::worker(),
            transport: BulkTransport::shared_memory_name(
                "Local\\locus-test-pixels",
                0,
                640 * 480 * 4,
            ),
        };
        let msg = WorkerToHost::PresentPixels {
            correlation_id: CorrelationId::new(3),
            session_id: SessionId::new(2),
            pane_id: PaneId::new(5),
            frame: PixelFrame {
                frame_id: 10,
                width: 640,
                height: 480,
                stride_bytes: 640 * 4,
                format: PixelFormat::Bgra8IgnoreAlpha,
                payload: bulk,
            },
        };
        let bytes = encode_worker_to_host(Seq::new(2), CorrelationId::new(3), &msg).unwrap();
        let (_, decoded) = decode_worker_to_host(&bytes).unwrap();
        assert_eq!(decoded, msg);
    }

    #[test]
    fn typed_worker_decode_rejects_invalid_bulk_payload() {
        let bulk = BulkDescriptor {
            bulk_id: BulkId::new(12),
            byte_len: 640 * 480 * 4,
            kind: BulkKind::TextBlob,
            access: BulkAccess::ReadOnly,
            generation: 3,
            trust: TrustLabel::worker(),
            transport: BulkTransport::shared_memory_name(
                "Local\\locus-test-pixels",
                0,
                640 * 480 * 4,
            ),
        };
        let msg = WorkerToHost::PresentPixels {
            correlation_id: CorrelationId::new(3),
            session_id: SessionId::new(2),
            pane_id: PaneId::new(5),
            frame: PixelFrame {
                frame_id: 10,
                width: 640,
                height: 480,
                stride_bytes: 640 * 4,
                format: PixelFormat::Bgra8IgnoreAlpha,
                payload: bulk,
            },
        };
        let bytes = encode_frame(
            FrameKind::WorkerToHost,
            Seq::new(2),
            CorrelationId::new(3),
            &msg,
        )
        .unwrap();

        assert!(matches!(
            decode_worker_to_host(&bytes),
            Err(FrameError::InvalidWorkerMessage(_))
        ));
    }

    #[test]
    fn typed_worker_decode_rejects_invalid_draw_batch() {
        let msg = WorkerToHost::DrawBatch {
            session_id: SessionId::new(2),
            pane_id: PaneId::new(5),
            batch: DrawBatch {
                frame_id: 1,
                commands: vec![DrawCmd::FillRect {
                    rect: Rect {
                        left: 0.0,
                        top: f32::NAN,
                        right: 10.0,
                        bottom: 10.0,
                    },
                    color: Color {
                        r: 1.0,
                        g: 0.0,
                        b: 0.0,
                        a: 1.0,
                    },
                }],
            },
        };
        let bytes = encode_frame(
            FrameKind::WorkerToHost,
            Seq::new(2),
            CorrelationId::new(0),
            &msg,
        )
        .unwrap();

        assert_eq!(
            decode_worker_to_host(&bytes),
            Err(FrameError::InvalidWorkerMessage(
                MessageValidationError::InvalidDrawBatch(
                    DrawBatchValidationError::NonFiniteFloat { field: "rect.top" }
                )
            ))
        );
    }

    #[test]
    fn added_message_families_roundtrip() {
        let bulk_descriptor = BulkDescriptor {
            bulk_id: BulkId::new(8),
            byte_len: 4096,
            kind: BulkKind::PresentPixels,
            access: BulkAccess::ReadWrite,
            generation: 1,
            trust: TrustLabel::worker(),
            transport: BulkTransport::shared_memory_name("Local\\locus-test-bulk", 0, 4096),
        };
        let table_descriptor = BulkDescriptor {
            bulk_id: BulkId::new(9),
            byte_len: 64,
            kind: BulkKind::TablePage,
            access: BulkAccess::ReadOnly,
            generation: 1,
            trust: TrustLabel::worker(),
            transport: BulkTransport::shared_memory_name("Local\\locus-test-table", 0, 64),
        };
        let worker_messages = vec![
            WorkerToHost::OpenPane {
                correlation_id: CorrelationId::new(6),
                session_id: SessionId::new(1),
                title: "demo".to_string(),
                width: 640,
                height: 480,
            },
            WorkerToHost::OpenShaderPane {
                correlation_id: CorrelationId::new(14),
                session_id: SessionId::new(1),
                title: "shader".to_string(),
                width: 320,
                height: 200,
                source: ShaderSource::hlsl_main_image(
                    "float4 main_image(float2 p){return float4(1,0,0,1);}",
                ),
                trust: TrustLabel::worker(),
            },
            WorkerToHost::ClosePane {
                correlation_id: CorrelationId::new(7),
                session_id: SessionId::new(1),
                pane_id: PaneId::new(2),
            },
            WorkerToHost::RequestTimer {
                session_id: SessionId::new(1),
                pane_id: PaneId::new(2),
                timer_id: TimerId::new(3),
                interval_ms: 16,
                repeat: true,
            },
            WorkerToHost::CancelTimer {
                session_id: SessionId::new(1),
                pane_id: PaneId::new(2),
                timer_id: TimerId::new(3),
            },
            WorkerToHost::DrawBatch {
                session_id: SessionId::new(1),
                pane_id: PaneId::new(2),
                batch: DrawBatch {
                    frame_id: 99,
                    commands: vec![
                        DrawCmd::Clear(Color {
                            r: 0.0,
                            g: 0.0,
                            b: 0.0,
                            a: 1.0,
                        }),
                        DrawCmd::FillRect {
                            rect: Rect {
                                left: 1.0,
                                top: 2.0,
                                right: 3.0,
                                bottom: 4.0,
                            },
                            color: Color {
                                r: 1.0,
                                g: 0.0,
                                b: 0.0,
                                a: 1.0,
                            },
                        },
                    ],
                },
            },
            WorkerToHost::LogLine {
                session_id: SessionId::new(1),
                text: "hello".to_string(),
                trust: TrustLabel::worker(),
            },
            WorkerToHost::ConsolePrompt {
                correlation_id: CorrelationId::new(4),
                session_id: SessionId::new(1),
                prompt: "> ".to_string(),
            },
            WorkerToHost::RequestPlugin {
                correlation_id: CorrelationId::new(5),
                session_id: SessionId::new(1),
                provider: "sqlite".to_string(),
                operation: "query".to_string(),
                args: vec![1, 2, 3],
                payload: None,
                trust: TrustLabel::worker(),
            },
            WorkerToHost::AllocateBulk {
                correlation_id: CorrelationId::new(8),
                session_id: SessionId::new(1),
                pane_id: PaneId::new(2),
                byte_len: 4096,
                kind: BulkKind::PresentPixels,
                access: BulkAccess::ReadWrite,
                trust: TrustLabel::worker(),
            },
            WorkerToHost::ReleaseBulk {
                correlation_id: CorrelationId::new(9),
                session_id: SessionId::new(1),
                bulk_id: BulkId::new(8),
            },
            WorkerToHost::DbTableSetPageData {
                correlation_id: CorrelationId::new(11),
                session_id: SessionId::new(1),
                handle: 99,
                request_id: 12,
                payload: table_descriptor.clone(),
            },
            WorkerToHost::GetDpi {
                correlation_id: CorrelationId::new(12),
                session_id: SessionId::new(1),
                pane_id: PaneId::new(2),
            },
            WorkerToHost::SetShaderUniforms {
                correlation_id: CorrelationId::new(15),
                session_id: SessionId::new(1),
                pane_id: PaneId::new(2),
                updates: vec![ShaderUniformUpdate {
                    name: "gain".to_string(),
                    value: ShaderUniformValue::Float(0.5),
                }],
                trust: TrustLabel::worker(),
            },
            WorkerToHost::SetShaderRedrawRate {
                correlation_id: CorrelationId::new(16),
                session_id: SessionId::new(1),
                pane_id: PaneId::new(2),
                interval_ms: crate::shader::MIN_SHADER_REDRAW_INTERVAL_MS,
            },
            WorkerToHost::SetShaderPlayback {
                correlation_id: CorrelationId::new(17),
                session_id: SessionId::new(1),
                pane_id: PaneId::new(2),
                state: ShaderPlaybackState::Playing,
            },
            WorkerToHost::SetCursor {
                session_id: SessionId::new(1),
                pane_id: PaneId::new(2),
                kind: 3,
            },
            WorkerToHost::SystemColor {
                correlation_id: CorrelationId::new(13),
                session_id: SessionId::new(1),
                kind: 0,
            },
            WorkerToHost::AnalysisReport {
                session_id: SessionId::new(1),
                markdown: "# Analysis\n".to_string(),
            },
            WorkerToHost::CheckDiagnostics {
                session_id: SessionId::new(1),
                diagnostics: vec![crate::message::Diagnostic {
                    line: 1,
                    column: 2,
                    message: "RN-E0000: demo".to_string(),
                }],
            },
            WorkerToHost::LoweringResult {
                session_id: SessionId::new(1),
                title: "ANF".to_string(),
                text: "main = 42\n".to_string(),
                lang: "text".to_string(),
            },
        ];

        for (idx, msg) in worker_messages.into_iter().enumerate() {
            let correlation_id = worker_message_correlation(&msg).unwrap_or_default();
            let bytes =
                encode_worker_to_host(Seq::new(idx as u64 + 1), correlation_id, &msg).unwrap();
            let (_, decoded) = decode_worker_to_host(&bytes).unwrap();
            assert_eq!(decoded, msg);
        }

        let host_messages = vec![
            HostToWorker::InputEvent {
                session_id: SessionId::new(1),
                pane_id: PaneId::new(2),
                event: crate::event::UiEvent::Menu { item_id: 10 },
            },
            HostToWorker::TimerTick {
                session_id: SessionId::new(1),
                pane_id: PaneId::new(2),
                timer_id: TimerId::new(3),
                time_ms: 123,
            },
            HostToWorker::ResizePane {
                session_id: SessionId::new(1),
                pane_id: PaneId::new(2),
                width: 800,
                height: 600,
                dpi: 96,
            },
            HostToWorker::ClosePane {
                session_id: SessionId::new(1),
                pane_id: PaneId::new(2),
            },
            HostToWorker::PaneOpened {
                correlation_id: CorrelationId::new(4),
                session_id: SessionId::new(1),
                pane_id: PaneId::new(2),
            },
            HostToWorker::PaneOpenFailed {
                correlation_id: CorrelationId::new(5),
                session_id: SessionId::new(1),
                message: "unable to create pane".to_string(),
            },
            HostToWorker::PaneCloseResult {
                correlation_id: CorrelationId::new(6),
                session_id: SessionId::new(1),
                pane_id: PaneId::new(2),
                ok: true,
            },
            HostToWorker::ConsoleInput {
                correlation_id: CorrelationId::new(11),
                session_id: SessionId::new(1),
                line: "Ada".to_string(),
                trust: TrustLabel::external("test-console"),
            },
            HostToWorker::BulkAllocated {
                correlation_id: CorrelationId::new(7),
                session_id: SessionId::new(1),
                descriptor: bulk_descriptor,
            },
            HostToWorker::BulkAllocationFailed {
                correlation_id: CorrelationId::new(8),
                session_id: SessionId::new(1),
                message: "out of bulk memory".to_string(),
            },
            HostToWorker::BulkReleased {
                correlation_id: CorrelationId::new(9),
                session_id: SessionId::new(1),
                bulk_id: BulkId::new(8),
                ok: true,
            },
            HostToWorker::PixelFramePresented {
                correlation_id: CorrelationId::new(10),
                session_id: SessionId::new(1),
                pane_id: PaneId::new(2),
                frame_id: 99,
                ok: true,
                message: String::new(),
            },
            HostToWorker::DpiResult {
                correlation_id: CorrelationId::new(12),
                session_id: SessionId::new(1),
                pane_id: PaneId::new(2),
                ok: true,
                dpi_x: 144,
                dpi_y: 144,
            },
            HostToWorker::SystemColorResult {
                correlation_id: CorrelationId::new(13),
                session_id: SessionId::new(1),
                color: Color {
                    r: 0.9,
                    g: 0.8,
                    b: 0.7,
                    a: 1.0,
                },
            },
            HostToWorker::ShaderOpResult {
                correlation_id: CorrelationId::new(14),
                session_id: SessionId::new(1),
                pane_id: PaneId::new(2),
                ok: true,
                code: "ok".to_string(),
            },
        ];

        for (idx, msg) in host_messages.into_iter().enumerate() {
            let correlation_id = host_message_correlation(&msg).unwrap_or_default();
            let bytes =
                encode_host_to_worker(Seq::new(idx as u64 + 1), correlation_id, &msg).unwrap();
            let (_, decoded) = decode_host_to_worker(&bytes).unwrap();
            assert_eq!(decoded, msg);
        }
    }

    #[test]
    fn open_pane_allows_zero_dimensions_for_host_default_size() {
        let msg = WorkerToHost::OpenPane {
            correlation_id: CorrelationId::new(6),
            session_id: SessionId::new(1),
            title: "default size".to_string(),
            width: 0,
            height: 0,
        };

        let bytes = encode_worker_to_host(Seq::new(1), CorrelationId::new(6), &msg).unwrap();
        let (_, decoded) = decode_worker_to_host(&bytes).unwrap();
        assert_eq!(decoded, msg);
    }

    #[test]
    fn typed_encode_rejects_header_payload_correlation_mismatch() {
        let msg = WorkerToHost::OpenPane {
            correlation_id: CorrelationId::new(6),
            session_id: SessionId::new(1),
            title: "demo".to_string(),
            width: 640,
            height: 480,
        };

        assert_eq!(
            encode_worker_to_host(Seq::new(1), CorrelationId::new(7), &msg),
            Err(FrameError::CorrelationMismatch {
                header: CorrelationId::new(7),
                message: CorrelationId::new(6),
            })
        );
    }

    #[test]
    fn typed_decode_rejects_header_payload_correlation_mismatch() {
        let msg = HostToWorker::PaneOpened {
            correlation_id: CorrelationId::new(6),
            session_id: SessionId::new(1),
            pane_id: PaneId::new(2),
        };
        let bytes = encode_frame(
            FrameKind::HostToWorker,
            Seq::new(1),
            CorrelationId::new(7),
            &msg,
        )
        .unwrap();

        assert_eq!(
            decode_host_to_worker(&bytes),
            Err(FrameError::CorrelationMismatch {
                header: CorrelationId::new(7),
                message: CorrelationId::new(6),
            })
        );
    }

    #[test]
    fn rejects_truncated_header() {
        assert_eq!(
            decode_frame(&[0; 4]),
            Err(FrameError::TruncatedHeader { actual: 4 })
        );
    }

    #[test]
    fn rejects_truncated_payload() {
        let msg = HostToWorker::Shutdown {
            reason: StopReason::HostShutdown,
        };
        let mut bytes = encode_host_to_worker(Seq::new(1), CorrelationId::new(1), &msg).unwrap();
        bytes.pop();
        assert!(matches!(
            decode_frame(&bytes),
            Err(FrameError::TruncatedPayload { .. })
        ));
    }

    #[test]
    fn rejects_trailing_bytes() {
        let msg = HostToWorker::Shutdown {
            reason: StopReason::HostShutdown,
        };
        let mut bytes = encode_host_to_worker(Seq::new(1), CorrelationId::new(1), &msg).unwrap();
        bytes.push(0);
        assert!(matches!(
            decode_frame(&bytes),
            Err(FrameError::TrailingBytes { .. })
        ));
    }

    #[test]
    fn rejects_unsupported_major_version() {
        let msg = HostToWorker::Shutdown {
            reason: StopReason::HostShutdown,
        };
        let mut bytes = encode_host_to_worker(Seq::new(1), CorrelationId::new(1), &msg).unwrap();
        bytes[0] = 2;
        assert!(matches!(
            decode_frame(&bytes),
            Err(FrameError::UnsupportedMajor { .. })
        ));
    }

    #[test]
    fn rejects_unsupported_minor_version() {
        let msg = HostToWorker::Shutdown {
            reason: StopReason::HostShutdown,
        };
        let mut bytes = encode_host_to_worker(Seq::new(1), CorrelationId::new(1), &msg).unwrap();
        bytes[2..4].copy_from_slice(&(PROTOCOL_MINOR + 1).to_le_bytes());
        assert_eq!(
            decode_frame(&bytes),
            Err(FrameError::UnsupportedMinor {
                found: PROTOCOL_MINOR + 1,
                max: PROTOCOL_MINOR,
            })
        );
    }

    #[test]
    fn rejects_payload_length_over_max_from_header() {
        let header = FrameHeader {
            major: PROTOCOL_MAJOR,
            minor: PROTOCOL_MINOR,
            frame_kind: FrameKind::HostToWorker,
            flags: 0,
            payload_len: MAX_FRAME_PAYLOAD_BYTES + 1,
            seq: Seq::new(1),
            correlation_id: CorrelationId::new(0),
        };
        let mut bytes = Vec::new();
        write_header(&mut bytes, header);

        assert_eq!(
            decode_frame(&bytes),
            Err(FrameError::PayloadTooLarge {
                len: MAX_FRAME_PAYLOAD_BYTES + 1,
                max: MAX_FRAME_PAYLOAD_BYTES,
            })
        );
    }

    #[test]
    fn rejects_encoded_payload_over_max() {
        let msg = HostToWorker::StartSession {
            session_id: SessionId::new(1),
            kind: SessionKind::Run,
            source: "x".repeat(MAX_FRAME_PAYLOAD_BYTES as usize + 1),
        };

        assert!(matches!(
            encode_host_to_worker(Seq::new(1), CorrelationId::new(0), &msg),
            Err(FrameError::PayloadTooLarge { .. })
        ));
    }

    #[test]
    fn rejects_unknown_frame_kind() {
        let msg = HostToWorker::Shutdown {
            reason: StopReason::HostShutdown,
        };
        let mut bytes = encode_host_to_worker(Seq::new(1), CorrelationId::new(1), &msg).unwrap();
        bytes[4] = 99;
        assert_eq!(decode_frame(&bytes), Err(FrameError::UnknownFrameKind(99)));
    }

    #[test]
    fn rejects_wrong_message_direction() {
        let msg = HostToWorker::Shutdown {
            reason: StopReason::HostShutdown,
        };
        let bytes = encode_host_to_worker(Seq::new(1), CorrelationId::new(1), &msg).unwrap();
        assert!(matches!(
            decode_worker_to_host(&bytes),
            Err(FrameError::UnexpectedFrameKind { .. })
        ));
    }

    #[test]
    fn bulk_descriptor_validation_rejects_bad_lifetimes() {
        let mut descriptor = BulkDescriptor {
            bulk_id: BulkId::new(0),
            byte_len: 1,
            kind: BulkKind::TablePage,
            access: BulkAccess::ReadOnly,
            generation: 1,
            trust: TrustLabel::worker(),
            transport: BulkTransport::shared_memory_name("Local\\locus-test-table", 0, 1),
        };
        assert_eq!(descriptor.validate(), Err(BulkValidationError::ZeroId));

        descriptor.bulk_id = BulkId::new(1);
        descriptor.byte_len = 0;
        assert_eq!(descriptor.validate(), Err(BulkValidationError::Empty));

        descriptor.byte_len = MAX_BULK_BYTES + 1;
        assert_eq!(
            descriptor.validate(),
            Err(BulkValidationError::TooLarge {
                byte_len: MAX_BULK_BYTES + 1,
                max: MAX_BULK_BYTES,
            })
        );

        descriptor.byte_len = 1;
        descriptor.generation = 0;
        assert_eq!(
            descriptor.validate(),
            Err(BulkValidationError::ZeroGeneration)
        );
    }

    #[test]
    fn bulk_descriptor_validation_rejects_bad_shared_memory_transport() {
        let mut descriptor = BulkDescriptor {
            bulk_id: BulkId::new(1),
            byte_len: 64,
            kind: BulkKind::PresentPixels,
            access: BulkAccess::ReadOnly,
            generation: 1,
            trust: TrustLabel::worker(),
            transport: BulkTransport::shared_memory_name("", 0, 64),
        };
        assert_eq!(
            descriptor.validate(),
            Err(BulkValidationError::EmptyTransportName)
        );

        descriptor.transport =
            BulkTransport::shared_memory_name("Local\\locus-test-pixels", 32, 64);
        assert_eq!(
            descriptor.validate(),
            Err(BulkValidationError::TransportTooSmall {
                byte_offset: 32,
                byte_len: 64,
                mapped_len: 64,
            })
        );

        descriptor.transport =
            BulkTransport::shared_memory_name("Local\\locus-test-pixels", u64::MAX, 64);
        assert_eq!(
            descriptor.validate(),
            Err(BulkValidationError::TransportRangeOverflow {
                byte_offset: u64::MAX,
                byte_len: 64,
            })
        );
    }

    #[test]
    fn protocol_crate_has_no_forbidden_dependencies_or_raw_handle_fields() {
        let manifest = include_str!("../Cargo.toml");
        for forbidden in ["locus-igui", "windows", "winapi", "direct2d", "directwrite"] {
            assert!(
                !manifest.contains(forbidden),
                "forbidden dependency marker {forbidden}"
            );
        }

        let sources = [
            ("bulk.rs", include_str!("bulk.rs")),
            ("draw.rs", include_str!("draw.rs")),
            ("event.rs", include_str!("event.rs")),
            ("frame.rs", include_str!("frame.rs")),
            ("ids.rs", include_str!("ids.rs")),
            ("lib.rs", include_str!("lib.rs")),
            ("message.rs", include_str!("message.rs")),
            ("shader.rs", include_str!("shader.rs")),
            ("shader_policy.rs", include_str!("shader_policy.rs")),
            ("trust.rs", include_str!("trust.rs")),
            ("version.rs", include_str!("version.rs")),
        ];

        for (path, source) in sources {
            let raw_pointer_markers = [
                ["*", "const"].concat(),
                ["*", "mut"].concat(),
                ["Non", "Null<"].concat(),
                ["std", "::", "ptr"].concat(),
                ["core", "::", "ptr"].concat(),
            ];
            let window_handle_marker = ["h", "wnd"].concat();

            for (line_no, line) in source.lines().enumerate() {
                let trimmed = line.trim_start();
                if trimmed.starts_with("//") {
                    continue;
                }

                for forbidden in &raw_pointer_markers {
                    assert!(
                        !line.contains(forbidden.as_str()),
                        "{path}:{} contains raw pointer marker {forbidden}",
                        line_no + 1
                    );
                }

                let lower = line.to_ascii_lowercase();
                assert!(
                    !(lower.contains(window_handle_marker.as_str())
                        && (lower.contains(':') || lower.contains("pub "))),
                    "{path}:{} contains a raw window-handle-like field",
                    line_no + 1
                );
            }
        }
    }
}
