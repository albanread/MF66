use serde::{Deserialize, Serialize};

use crate::bulk::{BulkAccess, BulkDescriptor, BulkKind, BulkValidationError};

pub const MAX_DRAW_BATCH_COMMANDS: usize = 16_384;
pub const MAX_DRAW_TEXT_BYTES: usize = 16 * 1024;
pub const MAX_DRAW_COORD_ABS: f32 = 1_000_000.0;
pub const MAX_DRAW_THICKNESS: f32 = 10_000.0;
pub const MAX_DRAW_RADIUS: f32 = 1_000_000.0;
pub const MAX_PIXEL_FRAME_DIMENSION: u32 = 32_768;

#[derive(Clone, Copy, Debug, PartialEq, Deserialize, Serialize)]
pub struct Point {
    pub x: f32,
    pub y: f32,
}

#[derive(Clone, Copy, Debug, PartialEq, Deserialize, Serialize)]
pub struct Rect {
    pub left: f32,
    pub top: f32,
    pub right: f32,
    pub bottom: f32,
}

#[derive(Clone, Copy, Debug, PartialEq, Deserialize, Serialize)]
pub struct Color {
    pub r: f32,
    pub g: f32,
    pub b: f32,
    pub a: f32,
}

#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
pub enum DrawCmd {
    Clear(Color),
    FillRect {
        rect: Rect,
        color: Color,
    },
    StrokeRect {
        rect: Rect,
        color: Color,
        thickness: f32,
    },
    DrawLine {
        from: Point,
        to: Point,
        color: Color,
        thickness: f32,
    },
    FillOval {
        rect: Rect,
        color: Color,
    },
    FillCircle {
        center: Point,
        radius: f32,
        color: Color,
    },
    StrokeOval {
        rect: Rect,
        color: Color,
        thickness: f32,
    },
    StrokeCircle {
        center: Point,
        radius: f32,
        color: Color,
        thickness: f32,
    },
    DrawArc {
        center: Point,
        radius: f32,
        rotation_rad: f32,
        half_aperture_rad: f32,
        color: Color,
        thickness: f32,
    },
    DrawText {
        text: String,
        x: f32,
        y: f32,
        size: f32,
        color: Color,
    },
}

#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
pub struct DrawBatch {
    pub frame_id: u64,
    pub commands: Vec<DrawCmd>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum DrawBatchValidationError {
    ZeroFrameId,
    TooManyCommands {
        count: usize,
        max: usize,
    },
    NonFiniteFloat {
        field: &'static str,
    },
    CoordinateOutOfRange {
        field: &'static str,
        value: f32,
        max_abs: f32,
    },
    ColorOutOfRange {
        field: &'static str,
        value: f32,
    },
    InvalidThickness {
        value: f32,
        max: f32,
    },
    DrawTextTooLarge {
        bytes: usize,
        max: usize,
    },
    InvalidRadius {
        value: f32,
        max: f32,
    },
}

impl DrawBatch {
    pub fn validate(&self) -> Result<(), DrawBatchValidationError> {
        if self.frame_id == 0 {
            return Err(DrawBatchValidationError::ZeroFrameId);
        }
        if self.commands.len() > MAX_DRAW_BATCH_COMMANDS {
            return Err(DrawBatchValidationError::TooManyCommands {
                count: self.commands.len(),
                max: MAX_DRAW_BATCH_COMMANDS,
            });
        }
        for cmd in &self.commands {
            validate_draw_cmd(cmd)?;
        }
        Ok(())
    }
}

fn validate_draw_cmd(cmd: &DrawCmd) -> Result<(), DrawBatchValidationError> {
    match cmd {
        DrawCmd::Clear(color) => validate_color("clear.color", color),
        DrawCmd::FillRect { rect, color } => {
            validate_rect(rect)?;
            validate_color("fill_rect.color", color)
        }
        DrawCmd::StrokeRect {
            rect,
            color,
            thickness,
        } => {
            validate_rect(rect)?;
            validate_color("stroke_rect.color", color)?;
            validate_thickness(*thickness)
        }
        DrawCmd::DrawLine {
            from,
            to,
            color,
            thickness,
        } => {
            validate_point("draw_line.from", from)?;
            validate_point("draw_line.to", to)?;
            validate_color("draw_line.color", color)?;
            validate_thickness(*thickness)
        }
        DrawCmd::FillOval { rect, color } => {
            validate_rect(rect)?;
            validate_color("fill_oval.color", color)
        }
        DrawCmd::FillCircle {
            center,
            radius,
            color,
        } => {
            validate_point("fill_circle.center", center)?;
            validate_radius(*radius)?;
            validate_color("fill_circle.color", color)
        }
        DrawCmd::StrokeOval {
            rect,
            color,
            thickness,
        } => {
            validate_rect(rect)?;
            validate_color("stroke_oval.color", color)?;
            validate_thickness(*thickness)
        }
        DrawCmd::StrokeCircle {
            center,
            radius,
            color,
            thickness,
        } => {
            validate_point("stroke_circle.center", center)?;
            validate_radius(*radius)?;
            validate_color("stroke_circle.color", color)?;
            validate_thickness(*thickness)
        }
        DrawCmd::DrawArc {
            center,
            radius,
            rotation_rad,
            half_aperture_rad,
            color,
            thickness,
        } => {
            validate_point("draw_arc.center", center)?;
            validate_radius(*radius)?;
            validate_finite("draw_arc.rotation_rad", *rotation_rad)?;
            validate_finite("draw_arc.half_aperture_rad", *half_aperture_rad)?;
            validate_color("draw_arc.color", color)?;
            validate_thickness(*thickness)
        }
        DrawCmd::DrawText { text, .. } if text.len() > MAX_DRAW_TEXT_BYTES => {
            Err(DrawBatchValidationError::DrawTextTooLarge {
                bytes: text.len(),
                max: MAX_DRAW_TEXT_BYTES,
            })
        }
        DrawCmd::DrawText {
            x, y, size, color, ..
        } => {
            validate_coord("draw_text.x", *x)?;
            validate_coord("draw_text.y", *y)?;
            validate_thickness(*size)?;
            validate_color("draw_text.color", color)
        }
    }
}

fn validate_rect(rect: &Rect) -> Result<(), DrawBatchValidationError> {
    validate_coord("rect.left", rect.left)?;
    validate_coord("rect.top", rect.top)?;
    validate_coord("rect.right", rect.right)?;
    validate_coord("rect.bottom", rect.bottom)
}

fn validate_point(prefix: &'static str, point: &Point) -> Result<(), DrawBatchValidationError> {
    validate_coord(point_field(prefix, ".x"), point.x)?;
    validate_coord(point_field(prefix, ".y"), point.y)
}

fn point_field(prefix: &'static str, suffix: &'static str) -> &'static str {
    match (prefix, suffix) {
        ("draw_line.from", ".x") => "from.x",
        ("draw_line.from", ".y") => "from.y",
        ("draw_line.to", ".x") => "to.x",
        ("draw_line.to", ".y") => "to.y",
        ("fill_circle.center", ".x") => "fill_circle.center.x",
        ("fill_circle.center", ".y") => "fill_circle.center.y",
        ("stroke_circle.center", ".x") => "stroke_circle.center.x",
        ("stroke_circle.center", ".y") => "stroke_circle.center.y",
        ("draw_arc.center", ".x") => "draw_arc.center.x",
        ("draw_arc.center", ".y") => "draw_arc.center.y",
        _ => prefix,
    }
}

fn validate_color(prefix: &'static str, color: &Color) -> Result<(), DrawBatchValidationError> {
    validate_color_component(prefix, ".r", color.r)?;
    validate_color_component(prefix, ".g", color.g)?;
    validate_color_component(prefix, ".b", color.b)?;
    validate_color_component(prefix, ".a", color.a)
}

fn validate_color_component(
    prefix: &'static str,
    suffix: &'static str,
    value: f32,
) -> Result<(), DrawBatchValidationError> {
    validate_finite(prefix, value)?;
    if !(0.0..=1.0).contains(&value) {
        return Err(DrawBatchValidationError::ColorOutOfRange {
            field: suffix,
            value,
        });
    }
    Ok(())
}

fn validate_thickness(value: f32) -> Result<(), DrawBatchValidationError> {
    validate_finite("thickness", value)?;
    if !(0.0..=MAX_DRAW_THICKNESS).contains(&value) {
        return Err(DrawBatchValidationError::InvalidThickness {
            value,
            max: MAX_DRAW_THICKNESS,
        });
    }
    Ok(())
}

fn validate_radius(value: f32) -> Result<(), DrawBatchValidationError> {
    validate_finite("radius", value)?;
    if !(0.0..=MAX_DRAW_RADIUS).contains(&value) {
        return Err(DrawBatchValidationError::InvalidRadius {
            value,
            max: MAX_DRAW_RADIUS,
        });
    }
    Ok(())
}

fn validate_coord(field: &'static str, value: f32) -> Result<(), DrawBatchValidationError> {
    validate_finite(field, value)?;
    if value.abs() > MAX_DRAW_COORD_ABS {
        return Err(DrawBatchValidationError::CoordinateOutOfRange {
            field,
            value,
            max_abs: MAX_DRAW_COORD_ABS,
        });
    }
    Ok(())
}

fn validate_finite(field: &'static str, value: f32) -> Result<(), DrawBatchValidationError> {
    if !value.is_finite() {
        return Err(DrawBatchValidationError::NonFiniteFloat { field });
    }
    Ok(())
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Deserialize, Serialize)]
pub enum PixelFormat {
    Bgra8Premul,
    Bgra8IgnoreAlpha,
}

impl PixelFormat {
    pub const fn bytes_per_pixel(self) -> u32 {
        match self {
            Self::Bgra8Premul | Self::Bgra8IgnoreAlpha => 4,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
pub struct PixelFrame {
    pub frame_id: u64,
    pub width: u32,
    pub height: u32,
    pub stride_bytes: u32,
    pub format: PixelFormat,
    pub payload: BulkDescriptor,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum PixelFrameValidationError {
    Bulk(BulkValidationError),
    EmptyDimensions,
    DimensionsTooLarge {
        width: u32,
        height: u32,
        max_dimension: u32,
    },
    StrideTooSmall {
        stride_bytes: u32,
        min_stride_bytes: u32,
    },
    PayloadTooSmall {
        byte_len: u64,
        min_byte_len: u64,
    },
}

impl PixelFrame {
    pub fn validate(&self) -> Result<(), PixelFrameValidationError> {
        self.payload
            .validate_for(BulkKind::PresentPixels, BulkAccess::ReadOnly)
            .map_err(PixelFrameValidationError::Bulk)?;

        let min_byte_len = self.required_payload_bytes()?;
        if self.payload.byte_len < min_byte_len {
            return Err(PixelFrameValidationError::PayloadTooSmall {
                byte_len: self.payload.byte_len,
                min_byte_len,
            });
        }

        Ok(())
    }

    pub fn required_payload_bytes(&self) -> Result<u64, PixelFrameValidationError> {
        if self.width == 0 || self.height == 0 {
            return Err(PixelFrameValidationError::EmptyDimensions);
        }
        if self.width > MAX_PIXEL_FRAME_DIMENSION || self.height > MAX_PIXEL_FRAME_DIMENSION {
            return Err(PixelFrameValidationError::DimensionsTooLarge {
                width: self.width,
                height: self.height,
                max_dimension: MAX_PIXEL_FRAME_DIMENSION,
            });
        }

        let min_stride_bytes = self.width * self.format.bytes_per_pixel();
        if self.stride_bytes < min_stride_bytes {
            return Err(PixelFrameValidationError::StrideTooSmall {
                stride_bytes: self.stride_bytes,
                min_stride_bytes,
            });
        }

        Ok(u64::from(self.stride_bytes) * u64::from(self.height))
    }
}
