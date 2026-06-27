use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

use crate::analysis::{self, AnalysisPhase, AnalysisSection};
use crate::bulk::{BulkAccess, BulkDescriptor, BulkKind, BulkValidationError};
use crate::db_table::{
    validate_text as validate_db_table_text, DbTableEvent, DbTableValidationError,
};
use crate::draw::{Color, DrawBatch, DrawBatchValidationError, PixelFrame};
use crate::event::UiEvent;
use crate::ids::{BulkId, CorrelationId, PaneId, SessionId, TaskId, TimerId};
use crate::shader::{self, ShaderPlaybackState, ShaderSource, ShaderUniformUpdate};
use crate::trust::TrustLabel;

pub const MAX_CONSOLE_INPUT_BYTES: usize = 64 * 1024;

#[derive(Clone, Copy, Debug, Eq, PartialEq, Deserialize, Serialize)]
pub enum SessionKind {
    Run,
    Repl,
    Check,
    Analyze,
    Lowering,
    LoweringLlvm,
    LoweringAsm,
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Deserialize, Serialize)]
pub struct CompilePolicy {
    pub boundary_modules: Vec<String>,
    pub plugin_ids: Vec<String>,
}

impl CompilePolicy {
    pub fn normalized(mut self) -> Self {
        self.boundary_modules.sort();
        self.boundary_modules.dedup();
        self.plugin_ids.sort();
        self.plugin_ids.dedup();
        self
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
pub enum StopReason {
    User,
    Timeout,
    HostShutdown,
    WorkerUnresponsive,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ProtocolIdField {
    SessionId,
    PaneId,
    TaskId,
    BulkId,
    CorrelationId,
    TimerId,
    DbTableHandle,
}

#[derive(Clone, Debug, PartialEq)]
pub enum MessageValidationError {
    InvalidId { field: ProtocolIdField },
    EmptyField { field: &'static str },
    InvalidDimension { field: &'static str },
    InvalidPixelFrame(crate::draw::PixelFrameValidationError),
    InvalidDrawBatch(DrawBatchValidationError),
    InvalidBulk(BulkValidationError),
    InvalidDbTable(DbTableValidationError),
    InvalidShader(crate::shader::ShaderValidationError),
    InvalidAnalysis(crate::analysis::AnalysisValidationError),
    InvalidByteLength { field: &'static str },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CorrelationValidationError {
    ZeroId,
    AlreadyPending { correlation_id: CorrelationId },
    Stale { correlation_id: CorrelationId },
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct CorrelationTracker {
    pending: BTreeSet<CorrelationId>,
    deferred: BTreeSet<CorrelationId>,
}

impl CorrelationTracker {
    pub fn begin_request(
        &mut self,
        correlation_id: CorrelationId,
    ) -> Result<(), CorrelationValidationError> {
        require_correlation(correlation_id)?;
        if !self.pending.insert(correlation_id) {
            return Err(CorrelationValidationError::AlreadyPending { correlation_id });
        }
        Ok(())
    }

    pub fn defer(
        &mut self,
        correlation_id: CorrelationId,
    ) -> Result<(), CorrelationValidationError> {
        require_correlation(correlation_id)?;
        if !self.pending.contains(&correlation_id) {
            return Err(CorrelationValidationError::Stale { correlation_id });
        }
        self.deferred.insert(correlation_id);
        Ok(())
    }

    pub fn complete(
        &mut self,
        correlation_id: CorrelationId,
    ) -> Result<(), CorrelationValidationError> {
        require_correlation(correlation_id)?;
        if !self.pending.remove(&correlation_id) {
            return Err(CorrelationValidationError::Stale { correlation_id });
        }
        self.deferred.remove(&correlation_id);
        Ok(())
    }

    pub fn cancel(
        &mut self,
        correlation_id: CorrelationId,
    ) -> Result<(), CorrelationValidationError> {
        self.complete(correlation_id)
    }

    pub fn is_pending(&self, correlation_id: CorrelationId) -> bool {
        self.pending.contains(&correlation_id)
    }

    pub fn is_deferred(&self, correlation_id: CorrelationId) -> bool {
        self.deferred.contains(&correlation_id)
    }
}

#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
pub enum HostToWorker {
    HandshakeAck {
        negotiated_minor: u16,
        features: u64,
    },
    SetCompilePolicy {
        policy: CompilePolicy,
    },
    StartSession {
        session_id: SessionId,
        kind: SessionKind,
        source: String,
    },
    StopSession {
        session_id: SessionId,
        reason: StopReason,
        grace_ms: u32,
    },
    InputEvent {
        session_id: SessionId,
        pane_id: PaneId,
        event: UiEvent,
    },
    TimerTick {
        session_id: SessionId,
        pane_id: PaneId,
        timer_id: TimerId,
        time_ms: u64,
    },
    ResizePane {
        session_id: SessionId,
        pane_id: PaneId,
        width: u32,
        height: u32,
        dpi: u32,
    },
    ClosePane {
        session_id: SessionId,
        pane_id: PaneId,
    },
    PaneOpened {
        correlation_id: CorrelationId,
        session_id: SessionId,
        pane_id: PaneId,
    },
    PaneOpenFailed {
        correlation_id: CorrelationId,
        session_id: SessionId,
        message: String,
    },
    PaneCloseResult {
        correlation_id: CorrelationId,
        session_id: SessionId,
        pane_id: PaneId,
        ok: bool,
    },
    /// Compatibility umbrella for routed UI events. Prefer the explicit
    /// InputEvent, TimerTick, ResizePane, and ClosePane families in new code.
    UiEvent {
        session_id: SessionId,
        pane_id: PaneId,
        event: UiEvent,
    },
    ConsoleInput {
        correlation_id: CorrelationId,
        session_id: SessionId,
        line: String,
        trust: TrustLabel,
    },
    PluginResult {
        correlation_id: CorrelationId,
        ok: bool,
        value: i64,
        message: String,
        trust: TrustLabel,
    },
    BulkAllocated {
        correlation_id: CorrelationId,
        session_id: SessionId,
        descriptor: BulkDescriptor,
    },
    BulkAllocationFailed {
        correlation_id: CorrelationId,
        session_id: SessionId,
        message: String,
    },
    BulkReleased {
        correlation_id: CorrelationId,
        session_id: SessionId,
        bulk_id: BulkId,
        ok: bool,
    },
    PixelFramePresented {
        correlation_id: CorrelationId,
        session_id: SessionId,
        pane_id: PaneId,
        frame_id: u64,
        ok: bool,
        message: String,
    },
    DbTableCreated {
        correlation_id: CorrelationId,
        session_id: SessionId,
        handle: i64,
    },
    DbTableCreateFailed {
        correlation_id: CorrelationId,
        session_id: SessionId,
        message: String,
    },
    DbTableOpResult {
        correlation_id: CorrelationId,
        session_id: SessionId,
        ok: bool,
    },
    DbTableEventResult {
        correlation_id: CorrelationId,
        session_id: SessionId,
        event: Option<DbTableEvent>,
    },
    DpiResult {
        correlation_id: CorrelationId,
        session_id: SessionId,
        pane_id: PaneId,
        ok: bool,
        dpi_x: u32,
        dpi_y: u32,
    },
    SystemColorResult {
        correlation_id: CorrelationId,
        session_id: SessionId,
        color: Color,
    },
    ShaderOpResult {
        correlation_id: CorrelationId,
        session_id: SessionId,
        pane_id: PaneId,
        ok: bool,
        code: String,
    },
    Ping {
        nonce: u64,
        host_time_ms: u64,
    },
    HeartbeatPing {
        nonce: u64,
        host_time_ms: u64,
    },
    DrawCredit {
        pane_id: PaneId,
        batch_credit: u32,
        pixel_credit: u32,
    },
    Shutdown {
        reason: StopReason,
    },
}

#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
pub enum WorkerToHost {
    Handshake {
        pid: u32,
        min_minor: u16,
        max_minor: u16,
        features: u64,
        build: String,
    },
    Ready,
    Heartbeat {
        nonce: u64,
        progress: u64,
        active_tasks: u32,
    },
    Pong {
        nonce: u64,
        pid: u32,
        protocol_minor: u16,
        build: String,
        progress: u64,
        active_tasks: u32,
    },
    OpenPane {
        correlation_id: CorrelationId,
        session_id: SessionId,
        title: String,
        /// Requested client width in pixels. `0` means host default.
        width: u32,
        /// Requested client height in pixels. `0` means host default.
        height: u32,
    },
    OpenShaderPane {
        correlation_id: CorrelationId,
        session_id: SessionId,
        title: String,
        width: u32,
        height: u32,
        source: ShaderSource,
        trust: TrustLabel,
    },
    ClosePane {
        correlation_id: CorrelationId,
        session_id: SessionId,
        pane_id: PaneId,
    },
    SetTitle {
        session_id: SessionId,
        pane_id: PaneId,
        title: String,
    },
    RequestTimer {
        session_id: SessionId,
        pane_id: PaneId,
        timer_id: TimerId,
        interval_ms: u32,
        repeat: bool,
    },
    CancelTimer {
        session_id: SessionId,
        pane_id: PaneId,
        timer_id: TimerId,
    },
    DrawBatch {
        session_id: SessionId,
        pane_id: PaneId,
        batch: DrawBatch,
    },
    PresentPixels {
        correlation_id: CorrelationId,
        session_id: SessionId,
        pane_id: PaneId,
        frame: PixelFrame,
    },
    SetShaderSource {
        correlation_id: CorrelationId,
        session_id: SessionId,
        pane_id: PaneId,
        source: ShaderSource,
        trust: TrustLabel,
    },
    SetShaderUniforms {
        correlation_id: CorrelationId,
        session_id: SessionId,
        pane_id: PaneId,
        updates: Vec<ShaderUniformUpdate>,
        trust: TrustLabel,
    },
    SetShaderRedrawRate {
        correlation_id: CorrelationId,
        session_id: SessionId,
        pane_id: PaneId,
        interval_ms: u32,
    },
    SetShaderPlayback {
        correlation_id: CorrelationId,
        session_id: SessionId,
        pane_id: PaneId,
        state: ShaderPlaybackState,
    },
    LogLine {
        session_id: SessionId,
        text: String,
        trust: TrustLabel,
    },
    ConsolePrompt {
        correlation_id: CorrelationId,
        session_id: SessionId,
        prompt: String,
    },
    RequestPlugin {
        correlation_id: CorrelationId,
        session_id: SessionId,
        provider: String,
        operation: String,
        args: Vec<i64>,
        payload: Option<BulkDescriptor>,
        trust: TrustLabel,
    },
    AllocateBulk {
        correlation_id: CorrelationId,
        session_id: SessionId,
        pane_id: PaneId,
        byte_len: u64,
        kind: BulkKind,
        access: BulkAccess,
        trust: TrustLabel,
    },
    ReleaseBulk {
        correlation_id: CorrelationId,
        session_id: SessionId,
        bulk_id: BulkId,
    },
    DbTableCreate {
        correlation_id: CorrelationId,
        session_id: SessionId,
        title: String,
        epoch: i64,
        row_count: i64,
    },
    DbTableAddColumn {
        correlation_id: CorrelationId,
        session_id: SessionId,
        handle: i64,
        id: String,
        title: String,
        kind: i64,
        width: f64,
        min_width: f64,
        max_width: f64,
        sortable: bool,
    },
    DbTableShow {
        correlation_id: CorrelationId,
        session_id: SessionId,
        handle: i64,
    },
    DbTablePollEvent {
        correlation_id: CorrelationId,
        session_id: SessionId,
        handle: i64,
        timeout_ms: i64,
    },
    DbTableSetModel {
        correlation_id: CorrelationId,
        session_id: SessionId,
        handle: i64,
        epoch: i64,
        row_count: i64,
    },
    DbTableSetCellText {
        correlation_id: CorrelationId,
        session_id: SessionId,
        handle: i64,
        row: i64,
        col: i64,
        text: String,
    },
    DbTableSetPageData {
        correlation_id: CorrelationId,
        session_id: SessionId,
        handle: i64,
        request_id: i64,
        payload: BulkDescriptor,
    },
    DbTablePageComplete {
        correlation_id: CorrelationId,
        session_id: SessionId,
        handle: i64,
        request_id: i64,
    },
    DbTableSetError {
        correlation_id: CorrelationId,
        session_id: SessionId,
        handle: i64,
        message: String,
    },
    DbTableClose {
        correlation_id: CorrelationId,
        session_id: SessionId,
        handle: i64,
    },
    GetDpi {
        correlation_id: CorrelationId,
        session_id: SessionId,
        pane_id: PaneId,
    },
    SetCursor {
        session_id: SessionId,
        pane_id: PaneId,
        kind: i32,
    },
    SystemColor {
        correlation_id: CorrelationId,
        session_id: SessionId,
        kind: i32,
    },
    /// Compatibility alias for LogLine used by the current in-process console
    /// bridge inventory.
    ConsoleWrite {
        session_id: SessionId,
        text: String,
        trust: TrustLabel,
    },
    /// Compatibility alias for ConsolePrompt used by the current in-process
    /// console bridge inventory.
    ConsoleReadRequest {
        correlation_id: CorrelationId,
        session_id: SessionId,
        prompt: String,
    },
    AnalysisStarted {
        session_id: SessionId,
    },
    AnalysisProgress {
        session_id: SessionId,
        phase: AnalysisPhase,
        completed: u32,
        total: u32,
    },
    AnalysisSection {
        session_id: SessionId,
        section: AnalysisSection,
    },
    AnalysisFinished {
        session_id: SessionId,
        ok: bool,
    },
    AnalysisCancelled {
        session_id: SessionId,
    },
    AnalysisReport {
        session_id: SessionId,
        markdown: String,
    },
    CheckDiagnostics {
        session_id: SessionId,
        diagnostics: Vec<Diagnostic>,
    },
    LoweringResult {
        session_id: SessionId,
        title: String,
        text: String,
        lang: String,
    },
    TaskSnapshot {
        session_id: SessionId,
        tasks: Vec<TaskSnapshot>,
    },
    SessionExited {
        session_id: SessionId,
        result: i64,
        ok: bool,
    },
    Fault {
        session_id: SessionId,
        code: String,
        message: String,
    },
}

#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
pub struct TaskSnapshot {
    pub task_id: TaskId,
    pub name: String,
    pub state: String,
    pub wait: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
pub struct Diagnostic {
    pub line: u32,
    pub column: u32,
    pub message: String,
}

impl HostToWorker {
    pub fn validate(&self) -> Result<(), MessageValidationError> {
        match self {
            Self::HandshakeAck { .. }
            | Self::SetCompilePolicy { .. }
            | Self::Ping { .. }
            | Self::HeartbeatPing { .. }
            | Self::Shutdown { .. } => Ok(()),
            Self::StartSession { session_id, .. } | Self::StopSession { session_id, .. } => {
                require_id(ProtocolIdField::SessionId, session_id.is_zero())
            }
            Self::InputEvent {
                session_id,
                pane_id,
                event,
            }
            | Self::UiEvent {
                session_id,
                pane_id,
                event,
            } => {
                require_id(ProtocolIdField::SessionId, session_id.is_zero())?;
                require_id(ProtocolIdField::PaneId, pane_id.is_zero())?;
                validate_ui_event(event)
            }
            Self::TimerTick {
                session_id,
                pane_id,
                timer_id,
                ..
            } => {
                require_id(ProtocolIdField::SessionId, session_id.is_zero())?;
                require_id(ProtocolIdField::PaneId, pane_id.is_zero())?;
                require_id(ProtocolIdField::TimerId, timer_id.is_zero())
            }
            Self::ResizePane {
                session_id,
                pane_id,
                width,
                height,
                dpi,
            } => {
                require_id(ProtocolIdField::SessionId, session_id.is_zero())?;
                require_id(ProtocolIdField::PaneId, pane_id.is_zero())?;
                require_nonzero_dimension("width", *width)?;
                require_nonzero_dimension("height", *height)?;
                require_nonzero_dimension("dpi", *dpi)
            }
            Self::ClosePane {
                session_id,
                pane_id,
            } => {
                require_id(ProtocolIdField::SessionId, session_id.is_zero())?;
                require_id(ProtocolIdField::PaneId, pane_id.is_zero())
            }
            Self::PaneOpened {
                correlation_id,
                session_id,
                pane_id,
            } => {
                require_id(ProtocolIdField::CorrelationId, correlation_id.is_zero())?;
                require_id(ProtocolIdField::SessionId, session_id.is_zero())?;
                require_id(ProtocolIdField::PaneId, pane_id.is_zero())
            }
            Self::PaneOpenFailed {
                correlation_id,
                session_id,
                ..
            } => {
                require_id(ProtocolIdField::CorrelationId, correlation_id.is_zero())?;
                require_id(ProtocolIdField::SessionId, session_id.is_zero())
            }
            Self::PaneCloseResult {
                correlation_id,
                session_id,
                pane_id,
                ..
            } => {
                require_id(ProtocolIdField::CorrelationId, correlation_id.is_zero())?;
                require_id(ProtocolIdField::SessionId, session_id.is_zero())?;
                require_id(ProtocolIdField::PaneId, pane_id.is_zero())
            }
            Self::ConsoleInput {
                correlation_id,
                session_id,
                line,
                ..
            } => {
                require_id(ProtocolIdField::CorrelationId, correlation_id.is_zero())?;
                require_id(ProtocolIdField::SessionId, session_id.is_zero())?;
                require_bounded_bytes("line", line, MAX_CONSOLE_INPUT_BYTES)
            }
            Self::PluginResult { correlation_id, .. } => {
                require_id(ProtocolIdField::CorrelationId, correlation_id.is_zero())
            }
            Self::BulkAllocated {
                correlation_id,
                session_id,
                descriptor,
            } => {
                require_id(ProtocolIdField::CorrelationId, correlation_id.is_zero())?;
                require_id(ProtocolIdField::SessionId, session_id.is_zero())?;
                descriptor
                    .validate()
                    .map_err(MessageValidationError::InvalidBulk)
            }
            Self::BulkAllocationFailed {
                correlation_id,
                session_id,
                message,
            } => {
                require_id(ProtocolIdField::CorrelationId, correlation_id.is_zero())?;
                require_id(ProtocolIdField::SessionId, session_id.is_zero())?;
                require_nonempty("message", message)
            }
            Self::BulkReleased {
                correlation_id,
                session_id,
                bulk_id,
                ..
            } => {
                require_id(ProtocolIdField::CorrelationId, correlation_id.is_zero())?;
                require_id(ProtocolIdField::SessionId, session_id.is_zero())?;
                require_id(ProtocolIdField::BulkId, bulk_id.is_zero())
            }
            Self::PixelFramePresented {
                correlation_id,
                session_id,
                pane_id,
                frame_id,
                ..
            } => {
                require_id(ProtocolIdField::CorrelationId, correlation_id.is_zero())?;
                require_id(ProtocolIdField::SessionId, session_id.is_zero())?;
                require_id(ProtocolIdField::PaneId, pane_id.is_zero())?;
                require_nonzero_u64("frame_id", *frame_id)
            }
            Self::DbTableCreated {
                correlation_id,
                session_id,
                handle,
            } => {
                require_id(ProtocolIdField::CorrelationId, correlation_id.is_zero())?;
                require_id(ProtocolIdField::SessionId, session_id.is_zero())?;
                require_positive_handle(*handle)
            }
            Self::DbTableCreateFailed {
                correlation_id,
                session_id,
                message,
            } => {
                require_id(ProtocolIdField::CorrelationId, correlation_id.is_zero())?;
                require_id(ProtocolIdField::SessionId, session_id.is_zero())?;
                require_nonempty("message", message)
            }
            Self::DbTableOpResult {
                correlation_id,
                session_id,
                ..
            }
            | Self::DbTableEventResult {
                correlation_id,
                session_id,
                ..
            } => {
                require_id(ProtocolIdField::CorrelationId, correlation_id.is_zero())?;
                require_id(ProtocolIdField::SessionId, session_id.is_zero())
            }
            Self::DpiResult {
                correlation_id,
                session_id,
                pane_id,
                ok,
                dpi_x,
                dpi_y,
            } => {
                require_id(ProtocolIdField::CorrelationId, correlation_id.is_zero())?;
                require_id(ProtocolIdField::SessionId, session_id.is_zero())?;
                require_id(ProtocolIdField::PaneId, pane_id.is_zero())?;
                if *ok {
                    require_nonzero_dimension("dpi_x", *dpi_x)?;
                    require_nonzero_dimension("dpi_y", *dpi_y)?;
                }
                Ok(())
            }
            Self::SystemColorResult {
                correlation_id,
                session_id,
                color,
            } => {
                require_id(ProtocolIdField::CorrelationId, correlation_id.is_zero())?;
                require_id(ProtocolIdField::SessionId, session_id.is_zero())?;
                validate_color(color)
            }
            Self::ShaderOpResult {
                correlation_id,
                session_id,
                pane_id,
                ok,
                code,
            } => {
                require_id(ProtocolIdField::CorrelationId, correlation_id.is_zero())?;
                require_id(ProtocolIdField::SessionId, session_id.is_zero())?;
                if *ok {
                    require_id(ProtocolIdField::PaneId, pane_id.is_zero())?;
                }
                shader::validate_status_code(code).map_err(MessageValidationError::InvalidShader)
            }
            Self::DrawCredit { pane_id, .. } => {
                require_id(ProtocolIdField::PaneId, pane_id.is_zero())
            }
        }
    }
}

impl WorkerToHost {
    pub fn validate(&self) -> Result<(), MessageValidationError> {
        match self {
            Self::Handshake { .. } | Self::Ready | Self::Heartbeat { .. } | Self::Pong { .. } => {
                Ok(())
            }
            Self::OpenPane {
                correlation_id,
                session_id,
                ..
            } => {
                require_id(ProtocolIdField::CorrelationId, correlation_id.is_zero())?;
                require_id(ProtocolIdField::SessionId, session_id.is_zero())
            }
            Self::OpenShaderPane {
                correlation_id,
                session_id,
                title,
                width,
                height,
                source,
                ..
            } => {
                require_id(ProtocolIdField::CorrelationId, correlation_id.is_zero())?;
                require_id(ProtocolIdField::SessionId, session_id.is_zero())?;
                shader::validate_title(title).map_err(MessageValidationError::InvalidShader)?;
                shader::validate_pane_dimensions(*width, *height)
                    .map_err(MessageValidationError::InvalidShader)?;
                source
                    .validate()
                    .map_err(MessageValidationError::InvalidShader)
            }
            Self::ClosePane {
                correlation_id,
                session_id,
                pane_id,
            } => {
                require_id(ProtocolIdField::CorrelationId, correlation_id.is_zero())?;
                require_id(ProtocolIdField::SessionId, session_id.is_zero())?;
                require_id(ProtocolIdField::PaneId, pane_id.is_zero())
            }
            Self::SetTitle {
                session_id,
                pane_id,
                ..
            } => {
                require_id(ProtocolIdField::SessionId, session_id.is_zero())?;
                require_id(ProtocolIdField::PaneId, pane_id.is_zero())
            }
            Self::DrawBatch {
                session_id,
                pane_id,
                batch,
            } => {
                require_id(ProtocolIdField::SessionId, session_id.is_zero())?;
                require_id(ProtocolIdField::PaneId, pane_id.is_zero())?;
                batch
                    .validate()
                    .map_err(MessageValidationError::InvalidDrawBatch)
            }
            Self::RequestTimer {
                session_id,
                pane_id,
                timer_id,
                interval_ms,
                ..
            } => {
                require_id(ProtocolIdField::SessionId, session_id.is_zero())?;
                require_id(ProtocolIdField::PaneId, pane_id.is_zero())?;
                require_id(ProtocolIdField::TimerId, timer_id.is_zero())?;
                require_nonzero_dimension("interval_ms", *interval_ms)
            }
            Self::CancelTimer {
                session_id,
                pane_id,
                timer_id,
            } => {
                require_id(ProtocolIdField::SessionId, session_id.is_zero())?;
                require_id(ProtocolIdField::PaneId, pane_id.is_zero())?;
                require_id(ProtocolIdField::TimerId, timer_id.is_zero())
            }
            Self::PresentPixels {
                correlation_id,
                session_id,
                pane_id,
                frame,
            } => {
                require_id(ProtocolIdField::CorrelationId, correlation_id.is_zero())?;
                require_id(ProtocolIdField::SessionId, session_id.is_zero())?;
                require_id(ProtocolIdField::PaneId, pane_id.is_zero())?;
                frame
                    .validate()
                    .map_err(MessageValidationError::InvalidPixelFrame)
            }
            Self::SetShaderSource {
                correlation_id,
                session_id,
                pane_id,
                source,
                ..
            } => {
                require_id(ProtocolIdField::CorrelationId, correlation_id.is_zero())?;
                require_id(ProtocolIdField::SessionId, session_id.is_zero())?;
                require_id(ProtocolIdField::PaneId, pane_id.is_zero())?;
                source
                    .validate()
                    .map_err(MessageValidationError::InvalidShader)
            }
            Self::SetShaderUniforms {
                correlation_id,
                session_id,
                pane_id,
                updates,
                ..
            } => {
                require_id(ProtocolIdField::CorrelationId, correlation_id.is_zero())?;
                require_id(ProtocolIdField::SessionId, session_id.is_zero())?;
                require_id(ProtocolIdField::PaneId, pane_id.is_zero())?;
                shader::validate_uniform_updates(updates)
                    .map_err(MessageValidationError::InvalidShader)
            }
            Self::SetShaderRedrawRate {
                correlation_id,
                session_id,
                pane_id,
                interval_ms,
            } => {
                require_id(ProtocolIdField::CorrelationId, correlation_id.is_zero())?;
                require_id(ProtocolIdField::SessionId, session_id.is_zero())?;
                require_id(ProtocolIdField::PaneId, pane_id.is_zero())?;
                shader::validate_redraw_interval(*interval_ms)
                    .map_err(MessageValidationError::InvalidShader)
            }
            Self::SetShaderPlayback {
                correlation_id,
                session_id,
                pane_id,
                ..
            } => {
                require_id(ProtocolIdField::CorrelationId, correlation_id.is_zero())?;
                require_id(ProtocolIdField::SessionId, session_id.is_zero())?;
                require_id(ProtocolIdField::PaneId, pane_id.is_zero())
            }
            Self::LogLine { session_id, .. } | Self::ConsoleWrite { session_id, .. } => {
                require_id(ProtocolIdField::SessionId, session_id.is_zero())
            }
            Self::ConsolePrompt {
                correlation_id,
                session_id,
                ..
            }
            | Self::ConsoleReadRequest {
                correlation_id,
                session_id,
                ..
            } => {
                require_id(ProtocolIdField::CorrelationId, correlation_id.is_zero())?;
                require_id(ProtocolIdField::SessionId, session_id.is_zero())
            }
            Self::RequestPlugin {
                correlation_id,
                session_id,
                provider,
                operation,
                payload,
                ..
            } => {
                require_id(ProtocolIdField::CorrelationId, correlation_id.is_zero())?;
                require_id(ProtocolIdField::SessionId, session_id.is_zero())?;
                require_nonempty("provider", provider)?;
                require_nonempty("operation", operation)?;
                if let Some(payload) = payload {
                    payload
                        .validate_for(BulkKind::PluginBlob, BulkAccess::ReadOnly)
                        .map_err(MessageValidationError::InvalidBulk)?;
                }
                Ok(())
            }
            Self::AllocateBulk {
                correlation_id,
                session_id,
                pane_id,
                byte_len,
                ..
            } => {
                require_id(ProtocolIdField::CorrelationId, correlation_id.is_zero())?;
                require_id(ProtocolIdField::SessionId, session_id.is_zero())?;
                require_id(ProtocolIdField::PaneId, pane_id.is_zero())?;
                require_bulk_byte_len("byte_len", *byte_len)
            }
            Self::ReleaseBulk {
                correlation_id,
                session_id,
                bulk_id,
            } => {
                require_id(ProtocolIdField::CorrelationId, correlation_id.is_zero())?;
                require_id(ProtocolIdField::SessionId, session_id.is_zero())?;
                require_id(ProtocolIdField::BulkId, bulk_id.is_zero())
            }
            Self::DbTableCreate {
                correlation_id,
                session_id,
                title,
                ..
            } => {
                require_id(ProtocolIdField::CorrelationId, correlation_id.is_zero())?;
                require_id(ProtocolIdField::SessionId, session_id.is_zero())?;
                validate_db_table_text("title", title)
                    .map_err(MessageValidationError::InvalidDbTable)
            }
            Self::DbTableAddColumn {
                correlation_id,
                session_id,
                handle,
                id,
                title,
                ..
            } => {
                require_id(ProtocolIdField::CorrelationId, correlation_id.is_zero())?;
                require_id(ProtocolIdField::SessionId, session_id.is_zero())?;
                require_positive_handle(*handle)?;
                validate_db_table_text("id", id).map_err(MessageValidationError::InvalidDbTable)?;
                validate_db_table_text("title", title)
                    .map_err(MessageValidationError::InvalidDbTable)
            }
            Self::DbTableShow {
                correlation_id,
                session_id,
                handle,
            }
            | Self::DbTablePollEvent {
                correlation_id,
                session_id,
                handle,
                ..
            }
            | Self::DbTableSetModel {
                correlation_id,
                session_id,
                handle,
                ..
            }
            | Self::DbTablePageComplete {
                correlation_id,
                session_id,
                handle,
                ..
            }
            | Self::DbTableClose {
                correlation_id,
                session_id,
                handle,
            } => {
                require_id(ProtocolIdField::CorrelationId, correlation_id.is_zero())?;
                require_id(ProtocolIdField::SessionId, session_id.is_zero())?;
                require_positive_handle(*handle)
            }
            Self::GetDpi {
                correlation_id,
                session_id,
                pane_id,
            } => {
                require_id(ProtocolIdField::CorrelationId, correlation_id.is_zero())?;
                require_id(ProtocolIdField::SessionId, session_id.is_zero())?;
                require_id(ProtocolIdField::PaneId, pane_id.is_zero())
            }
            Self::SetCursor {
                session_id,
                pane_id,
                ..
            } => {
                require_id(ProtocolIdField::SessionId, session_id.is_zero())?;
                require_id(ProtocolIdField::PaneId, pane_id.is_zero())
            }
            Self::SystemColor {
                correlation_id,
                session_id,
                ..
            } => {
                require_id(ProtocolIdField::CorrelationId, correlation_id.is_zero())?;
                require_id(ProtocolIdField::SessionId, session_id.is_zero())
            }
            Self::DbTableSetCellText {
                correlation_id,
                session_id,
                handle,
                text,
                ..
            } => {
                require_id(ProtocolIdField::CorrelationId, correlation_id.is_zero())?;
                require_id(ProtocolIdField::SessionId, session_id.is_zero())?;
                require_positive_handle(*handle)?;
                validate_db_table_text("text", text).map_err(MessageValidationError::InvalidDbTable)
            }
            Self::DbTableSetPageData {
                correlation_id,
                session_id,
                handle,
                payload,
                ..
            } => {
                require_id(ProtocolIdField::CorrelationId, correlation_id.is_zero())?;
                require_id(ProtocolIdField::SessionId, session_id.is_zero())?;
                require_positive_handle(*handle)?;
                payload
                    .validate_for(BulkKind::TablePage, BulkAccess::ReadOnly)
                    .map_err(MessageValidationError::InvalidBulk)
            }
            Self::DbTableSetError {
                correlation_id,
                session_id,
                handle,
                message,
            } => {
                require_id(ProtocolIdField::CorrelationId, correlation_id.is_zero())?;
                require_id(ProtocolIdField::SessionId, session_id.is_zero())?;
                require_positive_handle(*handle)?;
                validate_db_table_text("message", message)
                    .map_err(MessageValidationError::InvalidDbTable)
            }
            Self::TaskSnapshot { session_id, tasks } => {
                require_id(ProtocolIdField::SessionId, session_id.is_zero())?;
                for task in tasks {
                    require_id(ProtocolIdField::TaskId, task.task_id.is_zero())?;
                }
                Ok(())
            }
            Self::AnalysisStarted { session_id }
            | Self::AnalysisFinished { session_id, .. }
            | Self::AnalysisCancelled { session_id }
            | Self::AnalysisReport { session_id, .. }
            | Self::CheckDiagnostics { session_id, .. } => {
                require_id(ProtocolIdField::SessionId, session_id.is_zero())
            }
            Self::AnalysisProgress {
                session_id,
                completed,
                total,
                ..
            } => {
                require_id(ProtocolIdField::SessionId, session_id.is_zero())?;
                analysis::validate_progress(*completed, *total)
                    .map_err(MessageValidationError::InvalidAnalysis)
            }
            Self::AnalysisSection {
                session_id,
                section,
            } => {
                require_id(ProtocolIdField::SessionId, session_id.is_zero())?;
                section
                    .validate()
                    .map_err(MessageValidationError::InvalidAnalysis)
            }
            Self::LoweringResult {
                session_id,
                title,
                lang,
                ..
            } => {
                require_id(ProtocolIdField::SessionId, session_id.is_zero())?;
                require_nonempty("title", title)?;
                require_nonempty("lang", lang)
            }
            Self::SessionExited { session_id, .. } | Self::Fault { session_id, .. } => {
                require_id(ProtocolIdField::SessionId, session_id.is_zero())
            }
        }
    }
}

fn require_id(field: ProtocolIdField, is_zero: bool) -> Result<(), MessageValidationError> {
    if is_zero {
        return Err(MessageValidationError::InvalidId { field });
    }
    Ok(())
}

fn require_correlation(correlation_id: CorrelationId) -> Result<(), CorrelationValidationError> {
    if correlation_id.is_zero() {
        return Err(CorrelationValidationError::ZeroId);
    }
    Ok(())
}

fn require_nonzero_dimension(
    field: &'static str,
    value: u32,
) -> Result<(), MessageValidationError> {
    if value == 0 {
        return Err(MessageValidationError::InvalidDimension { field });
    }
    Ok(())
}

fn require_nonzero_u64(field: &'static str, value: u64) -> Result<(), MessageValidationError> {
    if value == 0 {
        return Err(MessageValidationError::InvalidDimension { field });
    }
    Ok(())
}

fn require_bulk_byte_len(field: &'static str, value: u64) -> Result<(), MessageValidationError> {
    if value == 0 || value > crate::bulk::MAX_BULK_BYTES {
        return Err(MessageValidationError::InvalidByteLength { field });
    }
    Ok(())
}

fn require_bounded_bytes(
    field: &'static str,
    value: &str,
    max: usize,
) -> Result<(), MessageValidationError> {
    if value.len() > max {
        return Err(MessageValidationError::InvalidByteLength { field });
    }
    Ok(())
}

fn require_positive_handle(value: i64) -> Result<(), MessageValidationError> {
    if value <= 0 {
        return Err(MessageValidationError::InvalidId {
            field: ProtocolIdField::DbTableHandle,
        });
    }
    Ok(())
}

fn require_nonempty(field: &'static str, value: &str) -> Result<(), MessageValidationError> {
    if value.is_empty() {
        return Err(MessageValidationError::EmptyField { field });
    }
    Ok(())
}

fn validate_color(color: &Color) -> Result<(), MessageValidationError> {
    validate_color_channel("color.r", color.r)?;
    validate_color_channel("color.g", color.g)?;
    validate_color_channel("color.b", color.b)?;
    validate_color_channel("color.a", color.a)
}

fn validate_color_channel(field: &'static str, value: f32) -> Result<(), MessageValidationError> {
    if !value.is_finite() {
        return Err(MessageValidationError::InvalidDrawBatch(
            DrawBatchValidationError::NonFiniteFloat { field },
        ));
    }
    if !(0.0..=1.0).contains(&value) {
        return Err(MessageValidationError::InvalidDrawBatch(
            DrawBatchValidationError::ColorOutOfRange { field, value },
        ));
    }
    Ok(())
}

fn validate_ui_event(event: &UiEvent) -> Result<(), MessageValidationError> {
    match event {
        UiEvent::Resize { width, height, dpi } => {
            require_nonzero_dimension("width", *width)?;
            require_nonzero_dimension("height", *height)?;
            require_nonzero_dimension("dpi", *dpi)
        }
        UiEvent::TimerTick { tick_id, .. } => require_id(ProtocolIdField::TimerId, *tick_id == 0),
        _ => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bulk::{BulkAccess, BulkKind, BulkTransport};
    use crate::draw::{PixelFormat, PixelFrame, PixelFrameValidationError};
    use crate::ids::BulkId;
    use crate::shader::{
        ShaderSource, ShaderUniformUpdate, ShaderUniformValue, ShaderValidationError,
        MAX_SHADER_PANE_DIMENSION, MIN_SHADER_REDRAW_INTERVAL_MS,
    };

    fn pixel_bulk(byte_len: u64) -> BulkDescriptor {
        BulkDescriptor {
            bulk_id: BulkId::new(7),
            byte_len,
            kind: BulkKind::PresentPixels,
            access: BulkAccess::ReadOnly,
            generation: 1,
            trust: TrustLabel::worker(),
            transport: BulkTransport::shared_memory_name("Local\\locus-test-pixels", 0, byte_len),
        }
    }

    #[test]
    fn present_pixels_validation_rejects_bad_pixel_frame() {
        let msg = WorkerToHost::PresentPixels {
            correlation_id: CorrelationId::new(3),
            session_id: SessionId::new(1),
            pane_id: PaneId::new(2),
            frame: PixelFrame {
                frame_id: 1,
                width: 4,
                height: 4,
                stride_bytes: 8,
                format: PixelFormat::Bgra8Premul,
                payload: pixel_bulk(64),
            },
        };

        assert_eq!(
            msg.validate(),
            Err(MessageValidationError::InvalidPixelFrame(
                PixelFrameValidationError::StrideTooSmall {
                    stride_bytes: 8,
                    min_stride_bytes: 16,
                }
            ))
        );
    }

    #[test]
    fn present_pixels_validation_rejects_wrong_bulk_kind() {
        let mut bulk = pixel_bulk(64);
        bulk.kind = BulkKind::TextBlob;
        let msg = WorkerToHost::PresentPixels {
            correlation_id: CorrelationId::new(3),
            session_id: SessionId::new(1),
            pane_id: PaneId::new(2),
            frame: PixelFrame {
                frame_id: 1,
                width: 4,
                height: 4,
                stride_bytes: 16,
                format: PixelFormat::Bgra8IgnoreAlpha,
                payload: bulk,
            },
        };

        assert_eq!(
            msg.validate(),
            Err(MessageValidationError::InvalidPixelFrame(
                PixelFrameValidationError::Bulk(BulkValidationError::WrongKind {
                    found: BulkKind::TextBlob,
                    expected: BulkKind::PresentPixels,
                })
            ))
        );
    }

    #[test]
    fn worker_and_host_messages_reject_invalid_ids() {
        let worker_msg = WorkerToHost::SetTitle {
            session_id: SessionId::new(0),
            pane_id: PaneId::new(2),
            title: "x".to_string(),
        };
        assert_eq!(
            worker_msg.validate(),
            Err(MessageValidationError::InvalidId {
                field: ProtocolIdField::SessionId
            })
        );

        let host_msg = HostToWorker::ClosePane {
            session_id: SessionId::new(1),
            pane_id: PaneId::new(0),
        };
        assert_eq!(
            host_msg.validate(),
            Err(MessageValidationError::InvalidId {
                field: ProtocolIdField::PaneId
            })
        );

        let timer_msg = WorkerToHost::RequestTimer {
            session_id: SessionId::new(1),
            pane_id: PaneId::new(2),
            timer_id: TimerId::new(0),
            interval_ms: 16,
            repeat: true,
        };
        assert_eq!(
            timer_msg.validate(),
            Err(MessageValidationError::InvalidId {
                field: ProtocolIdField::TimerId
            })
        );

        let bulk_msg = WorkerToHost::AllocateBulk {
            correlation_id: CorrelationId::new(1),
            session_id: SessionId::new(1),
            pane_id: PaneId::new(0),
            byte_len: 8,
            kind: BulkKind::PresentPixels,
            access: BulkAccess::ReadWrite,
            trust: TrustLabel::worker(),
        };
        assert_eq!(
            bulk_msg.validate(),
            Err(MessageValidationError::InvalidId {
                field: ProtocolIdField::PaneId
            })
        );
    }

    #[test]
    fn shader_messages_validate_source_dimensions_and_uniforms() {
        let valid = WorkerToHost::OpenShaderPane {
            correlation_id: CorrelationId::new(10),
            session_id: SessionId::new(1),
            title: "GPU".to_string(),
            width: 320,
            height: 200,
            source: ShaderSource::hlsl_main_image(
                "float4 main_image(float2 p){return float4(1,0,0,1);}",
            ),
            trust: TrustLabel::worker(),
        };
        assert_eq!(valid.validate(), Ok(()));

        let too_wide = WorkerToHost::OpenShaderPane {
            correlation_id: CorrelationId::new(10),
            session_id: SessionId::new(1),
            title: "GPU".to_string(),
            width: MAX_SHADER_PANE_DIMENSION + 1,
            height: 200,
            source: ShaderSource::hlsl_main_image(
                "float4 main_image(float2 p){return float4(1,0,0,1);}",
            ),
            trust: TrustLabel::worker(),
        };
        assert_eq!(
            too_wide.validate(),
            Err(MessageValidationError::InvalidShader(
                ShaderValidationError::InvalidDimension {
                    field: "width",
                    value: MAX_SHADER_PANE_DIMENSION + 1,
                    max: MAX_SHADER_PANE_DIMENSION,
                }
            ))
        );

        let bad_uniform = WorkerToHost::SetShaderUniforms {
            correlation_id: CorrelationId::new(11),
            session_id: SessionId::new(1),
            pane_id: PaneId::new(2),
            updates: vec![ShaderUniformUpdate {
                name: "bad".to_string(),
                value: ShaderUniformValue::Float(f32::INFINITY),
            }],
            trust: TrustLabel::worker(),
        };
        assert_eq!(
            bad_uniform.validate(),
            Err(MessageValidationError::InvalidShader(
                ShaderValidationError::NonFiniteFloat {
                    field: "uniform.float"
                }
            ))
        );

        let too_fast = WorkerToHost::SetShaderRedrawRate {
            correlation_id: CorrelationId::new(12),
            session_id: SessionId::new(1),
            pane_id: PaneId::new(2),
            interval_ms: MIN_SHADER_REDRAW_INTERVAL_MS - 1,
        };
        assert_eq!(
            too_fast.validate(),
            Err(MessageValidationError::InvalidShader(
                ShaderValidationError::RedrawIntervalTooSmall {
                    interval_ms: MIN_SHADER_REDRAW_INTERVAL_MS - 1,
                    min: MIN_SHADER_REDRAW_INTERVAL_MS,
                }
            ))
        );
    }

    #[test]
    fn request_plugin_validates_payload_descriptor() {
        let msg = WorkerToHost::RequestPlugin {
            correlation_id: CorrelationId::new(9),
            session_id: SessionId::new(1),
            provider: "sqlite".to_string(),
            operation: "query".to_string(),
            args: vec![1, 2],
            payload: Some(pixel_bulk(64)),
            trust: TrustLabel::worker(),
        };

        assert_eq!(
            msg.validate(),
            Err(MessageValidationError::InvalidBulk(
                BulkValidationError::WrongKind {
                    found: BulkKind::PresentPixels,
                    expected: BulkKind::PluginBlob,
                }
            ))
        );
    }

    #[test]
    fn analysis_messages_validate_session_and_payload() {
        let valid = WorkerToHost::AnalysisSection {
            session_id: SessionId::new(1),
            section: AnalysisSection::markdown(
                crate::analysis::AnalysisSectionKind::Effects,
                "Effects",
                "## Effects\n",
            ),
        };
        assert_eq!(valid.validate(), Ok(()));

        let stale = WorkerToHost::AnalysisStarted {
            session_id: SessionId::new(0),
        };
        assert_eq!(
            stale.validate(),
            Err(MessageValidationError::InvalidId {
                field: ProtocolIdField::SessionId
            })
        );

        let empty_title = WorkerToHost::AnalysisSection {
            session_id: SessionId::new(1),
            section: AnalysisSection::markdown(
                crate::analysis::AnalysisSectionKind::Effects,
                "",
                "## Effects\n",
            ),
        };
        assert!(matches!(
            empty_title.validate(),
            Err(MessageValidationError::InvalidAnalysis(
                crate::analysis::AnalysisValidationError::EmptyText {
                    field: "analysis section title"
                }
            ))
        ));

        let bad_progress = WorkerToHost::AnalysisProgress {
            session_id: SessionId::new(1),
            phase: AnalysisPhase::Render,
            completed: 2,
            total: 1,
        };
        assert!(matches!(
            bad_progress.validate(),
            Err(MessageValidationError::InvalidAnalysis(
                crate::analysis::AnalysisValidationError::InvalidProgress {
                    completed: 2,
                    total: 1
                }
            ))
        ));
    }

    #[test]
    fn console_input_validation_requires_session_and_bounds_line() {
        let valid = HostToWorker::ConsoleInput {
            correlation_id: CorrelationId::new(3),
            session_id: SessionId::new(1),
            line: "Ada".to_string(),
            trust: TrustLabel::external("test"),
        };
        assert_eq!(valid.validate(), Ok(()));

        let stale = HostToWorker::ConsoleInput {
            correlation_id: CorrelationId::new(3),
            session_id: SessionId::new(0),
            line: "Ada".to_string(),
            trust: TrustLabel::external("test"),
        };
        assert_eq!(
            stale.validate(),
            Err(MessageValidationError::InvalidId {
                field: ProtocolIdField::SessionId
            })
        );

        let oversized = HostToWorker::ConsoleInput {
            correlation_id: CorrelationId::new(3),
            session_id: SessionId::new(1),
            line: "x".repeat(MAX_CONSOLE_INPUT_BYTES + 1),
            trust: TrustLabel::external("test"),
        };
        assert_eq!(
            oversized.validate(),
            Err(MessageValidationError::InvalidByteLength { field: "line" })
        );
    }

    #[test]
    fn db_table_page_data_requires_table_page_readonly_bulk() {
        let mut payload = pixel_bulk(64);
        payload.kind = BulkKind::TablePage;
        let valid = WorkerToHost::DbTableSetPageData {
            correlation_id: CorrelationId::new(4),
            session_id: SessionId::new(1),
            handle: 99,
            request_id: 12,
            payload: payload.clone(),
        };
        assert_eq!(valid.validate(), Ok(()));

        let wrong_kind = WorkerToHost::DbTableSetPageData {
            correlation_id: CorrelationId::new(4),
            session_id: SessionId::new(1),
            handle: 99,
            request_id: 12,
            payload: pixel_bulk(64),
        };
        assert_eq!(
            wrong_kind.validate(),
            Err(MessageValidationError::InvalidBulk(
                BulkValidationError::WrongKind {
                    found: BulkKind::PresentPixels,
                    expected: BulkKind::TablePage,
                }
            ))
        );

        payload.access = BulkAccess::ReadWrite;
        let wrong_access = WorkerToHost::DbTableSetPageData {
            correlation_id: CorrelationId::new(4),
            session_id: SessionId::new(1),
            handle: 99,
            request_id: 12,
            payload,
        };
        assert_eq!(
            wrong_access.validate(),
            Err(MessageValidationError::InvalidBulk(
                BulkValidationError::WrongAccess {
                    found: BulkAccess::ReadWrite,
                    expected: BulkAccess::ReadOnly,
                }
            ))
        );
    }

    #[test]
    fn correlation_tracker_rejects_stale_ids_and_tracks_deferral() {
        let mut tracker = CorrelationTracker::default();
        let correlation_id = CorrelationId::new(77);

        assert_eq!(
            tracker.complete(correlation_id),
            Err(CorrelationValidationError::Stale { correlation_id })
        );
        tracker.begin_request(correlation_id).unwrap();
        tracker.defer(correlation_id).unwrap();
        assert!(tracker.is_pending(correlation_id));
        assert!(tracker.is_deferred(correlation_id));
        tracker.complete(correlation_id).unwrap();
        assert!(!tracker.is_pending(correlation_id));
        assert!(!tracker.is_deferred(correlation_id));
        assert_eq!(
            tracker.defer(correlation_id),
            Err(CorrelationValidationError::Stale { correlation_id })
        );
    }
}
