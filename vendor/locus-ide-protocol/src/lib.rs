//! UI-neutral protocol types for the Locus IDE host/worker split.
//!
//! This crate deliberately contains no HWNDs, Direct2D/DirectWrite types, raw
//! pointers, or `locus-igui` dependencies. It is the serializable contract
//! between the host broker and disposable Locus worker processes.

pub mod analysis;
pub mod bulk;
pub mod db_table;
pub mod draw;
pub mod event;
pub mod frame;
pub mod ids;
pub mod message;
pub mod plugin_host;
pub mod shader;
pub mod shader_policy;
pub mod trust;
pub mod version;

pub use analysis::{
    AnalysisPhase, AnalysisSection, AnalysisSectionKind, AnalysisValidationError,
    MAX_ANALYSIS_SECTION_MARKDOWN_BYTES, MAX_ANALYSIS_SECTION_TITLE_BYTES,
};
pub use bulk::{
    BulkAccess, BulkDescriptor, BulkKind, BulkTransport, BulkValidationError, MAX_BULK_BYTES,
};
pub use db_table::{
    DbTableEvent, DbTableEventKind, DbTableValidationError, MAX_DB_TABLE_TEXT_BYTES,
};
pub use draw::{
    Color, DrawBatch, DrawBatchValidationError, DrawCmd, PixelFormat, PixelFrame,
    PixelFrameValidationError, Point, Rect, MAX_DRAW_BATCH_COMMANDS, MAX_DRAW_COORD_ABS,
    MAX_DRAW_RADIUS, MAX_DRAW_TEXT_BYTES, MAX_DRAW_THICKNESS, MAX_PIXEL_FRAME_DIMENSION,
};
pub use event::{KeyState, MouseButton, MouseEvent, MouseOp, UiEvent};
pub use frame::{
    decode_frame, encode_frame, FrameDecode, FrameError, FrameHeader, FrameKind,
    MAX_FRAME_PAYLOAD_BYTES,
};
pub use ids::{BulkId, CorrelationId, PaneId, Seq, SessionId, TaskId, TimerId, WorkerId};
pub use message::{
    CompilePolicy, CorrelationTracker, CorrelationValidationError, Diagnostic, HostToWorker,
    MessageValidationError, ProtocolIdField, SessionKind, StopReason, WorkerToHost,
};
pub use shader::{
    ShaderEntry, ShaderPlaybackState, ShaderProfile, ShaderSource, ShaderUniformUpdate,
    ShaderUniformValue, ShaderValidationError, MAX_SHADER_PANE_DIMENSION, MAX_SHADER_SOURCE_BYTES,
    MAX_SHADER_STATUS_CODE_BYTES, MAX_SHADER_TITLE_BYTES, MAX_SHADER_UNIFORM_NAME_BYTES,
    MAX_SHADER_UNIFORM_UPDATES, MIN_SHADER_REDRAW_INTERVAL_MS,
};
pub use shader_policy::{ShaderPolicyError, MAX_SHADER_FOR_LOOP_BOUND};
pub use trust::{TaintOrigin, TrustLabel};
pub use version::{PROTOCOL_MAJOR, PROTOCOL_MINOR};
