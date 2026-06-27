//! UI-neutral structured analyzer model.
//!
//! This crate is deliberately separate from the IDE host/worker protocol. The
//! compiler/analyzer can emit stable section identities and lifecycle phases
//! without depending on IDE message framing.

use serde::{Deserialize, Serialize};

pub const MAX_ANALYSIS_SECTION_TITLE_BYTES: usize = 256;
pub const MAX_ANALYSIS_SECTION_MARKDOWN_BYTES: usize = 4 * 1024 * 1024;

#[derive(Clone, Copy, Debug, Eq, PartialEq, Deserialize, Serialize)]
pub enum AnalysisPhase {
    Analyze,
    Render,
    Complete,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Deserialize, Serialize)]
pub enum AnalysisSectionKind {
    Intro,
    Diagnostics,
    Effects,
    Performance,
    Functions,
    CallGraph,
    DataAccess,
    CompatibilityMarkdown,
}

#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
pub struct AnalysisSection {
    pub kind: AnalysisSectionKind,
    pub title: String,
    /// Transitional display/export payload. The section kind is the stable API;
    /// later revisions can add typed rows and graphs beside this.
    pub markdown: String,
}

impl AnalysisSection {
    pub fn markdown(
        kind: AnalysisSectionKind,
        title: impl Into<String>,
        markdown: impl Into<String>,
    ) -> Self {
        Self {
            kind,
            title: title.into(),
            markdown: markdown.into(),
        }
    }

    pub fn validate(&self) -> Result<(), AnalysisValidationError> {
        validate_text(
            "analysis section title",
            &self.title,
            MAX_ANALYSIS_SECTION_TITLE_BYTES,
        )?;
        validate_text(
            "analysis section markdown",
            &self.markdown,
            MAX_ANALYSIS_SECTION_MARKDOWN_BYTES,
        )
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum AnalysisValidationError {
    EmptyText {
        field: &'static str,
    },
    TextTooLong {
        field: &'static str,
        max_bytes: usize,
        actual_bytes: usize,
    },
    InvalidProgress {
        completed: u32,
        total: u32,
    },
}

pub fn validate_progress(completed: u32, total: u32) -> Result<(), AnalysisValidationError> {
    if completed > total {
        return Err(AnalysisValidationError::InvalidProgress { completed, total });
    }
    Ok(())
}

fn validate_text(
    field: &'static str,
    text: &str,
    max_bytes: usize,
) -> Result<(), AnalysisValidationError> {
    if text.is_empty() {
        return Err(AnalysisValidationError::EmptyText { field });
    }
    let actual_bytes = text.len();
    if actual_bytes > max_bytes {
        return Err(AnalysisValidationError::TextTooLong {
            field,
            max_bytes,
            actual_bytes,
        });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validates_section_text_bounds() {
        let section = AnalysisSection::markdown(AnalysisSectionKind::Effects, "Effects", "ok");
        assert_eq!(section.validate(), Ok(()));

        let empty_title = AnalysisSection::markdown(AnalysisSectionKind::Effects, "", "ok");
        assert_eq!(
            empty_title.validate(),
            Err(AnalysisValidationError::EmptyText {
                field: "analysis section title"
            })
        );
    }

    #[test]
    fn validates_progress_bounds() {
        assert_eq!(validate_progress(1, 2), Ok(()));
        assert_eq!(
            validate_progress(3, 2),
            Err(AnalysisValidationError::InvalidProgress {
                completed: 3,
                total: 2
            })
        );
    }
}
