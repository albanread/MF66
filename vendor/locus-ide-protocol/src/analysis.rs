//! Compatibility re-exports for structured analyzer service messages.
//!
//! The canonical analyzer model lives in `locus-analysis-model` so compiler
//! services can emit reports without depending on the IDE host/worker protocol.

pub use locus_analysis_model::{
    validate_progress, AnalysisPhase, AnalysisSection, AnalysisSectionKind,
    AnalysisValidationError, MAX_ANALYSIS_SECTION_MARKDOWN_BYTES, MAX_ANALYSIS_SECTION_TITLE_BYTES,
};
