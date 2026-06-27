use serde::{Deserialize, Serialize};

use crate::shader_policy::{self, ShaderPolicyError};

pub const MAX_SHADER_SOURCE_BYTES: usize = 256 * 1024;
pub const MAX_SHADER_TITLE_BYTES: usize = 256;
pub const MAX_SHADER_UNIFORM_NAME_BYTES: usize = 64;
pub const MAX_SHADER_UNIFORM_UPDATES: usize = 128;
pub const MAX_SHADER_PANE_DIMENSION: u32 = 8_192;
pub const MIN_SHADER_REDRAW_INTERVAL_MS: u32 = 8;
pub const MAX_SHADER_STATUS_CODE_BYTES: usize = 64;

#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
pub struct ShaderSource {
    pub text: String,
    pub entry: ShaderEntry,
    pub profile: ShaderProfile,
}

impl ShaderSource {
    pub fn hlsl_main_image(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            entry: ShaderEntry::MainImage,
            profile: ShaderProfile::HlslPs50,
        }
    }

    pub fn validate(&self) -> Result<(), ShaderValidationError> {
        let bytes = self.text.len();
        if bytes == 0 {
            return Err(ShaderValidationError::EmptySource);
        }
        if bytes > MAX_SHADER_SOURCE_BYTES {
            return Err(ShaderValidationError::TextTooLarge {
                field: "source",
                bytes,
                max: MAX_SHADER_SOURCE_BYTES,
            });
        }
        shader_policy::validate_fragment_shader_policy(&self.text)
            .map_err(ShaderValidationError::Policy)?;
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Deserialize, Serialize)]
pub enum ShaderEntry {
    MainImage,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Deserialize, Serialize)]
pub enum ShaderProfile {
    HlslPs50,
}

#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
pub enum ShaderUniformValue {
    Int(i32),
    Float(f32),
    Vec2([f32; 2]),
    Vec3([f32; 3]),
    Vec4([f32; 4]),
}

impl ShaderUniformValue {
    fn validate(&self) -> Result<(), ShaderValidationError> {
        match self {
            Self::Int(_) => Ok(()),
            Self::Float(value) => validate_finite("uniform.float", *value),
            Self::Vec2(values) => validate_finite_slice("uniform.vec2", values),
            Self::Vec3(values) => validate_finite_slice("uniform.vec3", values),
            Self::Vec4(values) => validate_finite_slice("uniform.vec4", values),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
pub struct ShaderUniformUpdate {
    pub name: String,
    pub value: ShaderUniformValue,
}

impl ShaderUniformUpdate {
    pub fn validate(&self) -> Result<(), ShaderValidationError> {
        validate_uniform_name(&self.name)?;
        self.value.validate()
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Deserialize, Serialize)]
pub enum ShaderPlaybackState {
    Playing,
    Paused,
    ResetTime,
}

impl ShaderPlaybackState {
    pub const PLAYING_CODE: i64 = 1;
    pub const PAUSED_CODE: i64 = 2;
    pub const RESET_TIME_CODE: i64 = 3;

    pub const fn from_code(code: i64) -> Option<Self> {
        match code {
            Self::PLAYING_CODE => Some(Self::Playing),
            Self::PAUSED_CODE => Some(Self::Paused),
            Self::RESET_TIME_CODE => Some(Self::ResetTime),
            _ => None,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum ShaderValidationError {
    EmptySource,
    EmptyUniformName,
    TextTooLarge {
        field: &'static str,
        bytes: usize,
        max: usize,
    },
    TooManyUniformUpdates {
        count: usize,
        max: usize,
    },
    NonFiniteFloat {
        field: &'static str,
    },
    InvalidDimension {
        field: &'static str,
        value: u32,
        max: u32,
    },
    RedrawIntervalTooSmall {
        interval_ms: u32,
        min: u32,
    },
    Policy(ShaderPolicyError),
}

pub fn validate_title(title: &str) -> Result<(), ShaderValidationError> {
    validate_bounded_text("title", title, MAX_SHADER_TITLE_BYTES)
}

pub fn validate_uniform_updates(
    updates: &[ShaderUniformUpdate],
) -> Result<(), ShaderValidationError> {
    if updates.len() > MAX_SHADER_UNIFORM_UPDATES {
        return Err(ShaderValidationError::TooManyUniformUpdates {
            count: updates.len(),
            max: MAX_SHADER_UNIFORM_UPDATES,
        });
    }
    for update in updates {
        update.validate()?;
    }
    Ok(())
}

pub fn validate_pane_dimensions(width: u32, height: u32) -> Result<(), ShaderValidationError> {
    validate_pane_dimension("width", width)?;
    validate_pane_dimension("height", height)
}

pub fn validate_redraw_interval(interval_ms: u32) -> Result<(), ShaderValidationError> {
    if interval_ms < MIN_SHADER_REDRAW_INTERVAL_MS {
        return Err(ShaderValidationError::RedrawIntervalTooSmall {
            interval_ms,
            min: MIN_SHADER_REDRAW_INTERVAL_MS,
        });
    }
    Ok(())
}

pub fn validate_status_code(code: &str) -> Result<(), ShaderValidationError> {
    validate_bounded_text("code", code, MAX_SHADER_STATUS_CODE_BYTES)
}

fn validate_uniform_name(name: &str) -> Result<(), ShaderValidationError> {
    if name.is_empty() {
        return Err(ShaderValidationError::EmptyUniformName);
    }
    validate_bounded_text("uniform.name", name, MAX_SHADER_UNIFORM_NAME_BYTES)
}

fn validate_pane_dimension(field: &'static str, value: u32) -> Result<(), ShaderValidationError> {
    if value == 0 || value > MAX_SHADER_PANE_DIMENSION {
        return Err(ShaderValidationError::InvalidDimension {
            field,
            value,
            max: MAX_SHADER_PANE_DIMENSION,
        });
    }
    Ok(())
}

fn validate_bounded_text(
    field: &'static str,
    value: &str,
    max: usize,
) -> Result<(), ShaderValidationError> {
    let bytes = value.len();
    if bytes > max {
        return Err(ShaderValidationError::TextTooLarge { field, bytes, max });
    }
    Ok(())
}

fn validate_finite_slice(field: &'static str, values: &[f32]) -> Result<(), ShaderValidationError> {
    for value in values {
        validate_finite(field, *value)?;
    }
    Ok(())
}

fn validate_finite(field: &'static str, value: f32) -> Result<(), ShaderValidationError> {
    if !value.is_finite() {
        return Err(ShaderValidationError::NonFiniteFloat { field });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shader_source_enforces_byte_limit() {
        let valid = ShaderSource::hlsl_main_image("float4 main_image(float2 p){return 1;}");
        assert_eq!(valid.validate(), Ok(()));

        let empty = ShaderSource::hlsl_main_image("");
        assert_eq!(empty.validate(), Err(ShaderValidationError::EmptySource));

        let oversized = ShaderSource::hlsl_main_image("x".repeat(MAX_SHADER_SOURCE_BYTES + 1));
        assert_eq!(
            oversized.validate(),
            Err(ShaderValidationError::TextTooLarge {
                field: "source",
                bytes: MAX_SHADER_SOURCE_BYTES + 1,
                max: MAX_SHADER_SOURCE_BYTES,
            })
        );

        let forbidden =
            ShaderSource::hlsl_main_image("Texture2D data; float4 main_image(float2 p){return 1;}");
        assert_eq!(
            forbidden.validate(),
            Err(ShaderValidationError::Policy(
                ShaderPolicyError::ForbiddenIdentifier {
                    ident: "Texture2D".to_string(),
                    offset: 0,
                }
            ))
        );
    }

    #[test]
    fn uniform_updates_require_bounded_names_and_finite_values() {
        let valid = ShaderUniformUpdate {
            name: "gain".to_string(),
            value: ShaderUniformValue::Vec3([0.1, 0.2, 0.3]),
        };
        assert_eq!(valid.validate(), Ok(()));

        let empty_name = ShaderUniformUpdate {
            name: String::new(),
            value: ShaderUniformValue::Int(1),
        };
        assert_eq!(
            empty_name.validate(),
            Err(ShaderValidationError::EmptyUniformName)
        );

        let nan = ShaderUniformUpdate {
            name: "bad".to_string(),
            value: ShaderUniformValue::Float(f32::NAN),
        };
        assert_eq!(
            nan.validate(),
            Err(ShaderValidationError::NonFiniteFloat {
                field: "uniform.float"
            })
        );
    }

    #[test]
    fn pane_dimensions_and_redraw_rate_are_capped() {
        assert_eq!(validate_pane_dimensions(800, 600), Ok(()));
        assert_eq!(
            validate_pane_dimensions(0, 600),
            Err(ShaderValidationError::InvalidDimension {
                field: "width",
                value: 0,
                max: MAX_SHADER_PANE_DIMENSION,
            })
        );

        assert_eq!(
            validate_redraw_interval(MIN_SHADER_REDRAW_INTERVAL_MS - 1),
            Err(ShaderValidationError::RedrawIntervalTooSmall {
                interval_ms: MIN_SHADER_REDRAW_INTERVAL_MS - 1,
                min: MIN_SHADER_REDRAW_INTERVAL_MS,
            })
        );
    }
}
