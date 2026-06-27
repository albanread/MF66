use serde::{Deserialize, Serialize};

pub const MAX_DB_TABLE_TEXT_BYTES: usize = 16 * 1024;

#[derive(Clone, Copy, Debug, Eq, PartialEq, Deserialize, Serialize)]
pub enum DbTableEventKind {
    PageRequest,
    SortRequest,
    ColumnResize,
    Closed,
}

impl DbTableEventKind {
    pub const fn tag(self) -> i64 {
        match self {
            Self::PageRequest => 1,
            Self::SortRequest => 2,
            Self::ColumnResize => 3,
            Self::Closed => 4,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Deserialize, Serialize)]
pub struct DbTableEvent {
    pub kind: DbTableEventKind,
    pub request_id: i64,
    pub epoch: i64,
    pub row_start: i64,
    pub row_count: i64,
    pub col_start: i64,
    pub col_count: i64,
    pub value: i64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum DbTableValidationError {
    TextTooLarge {
        field: &'static str,
        bytes: usize,
        max: usize,
    },
}

pub fn validate_text(field: &'static str, value: &str) -> Result<(), DbTableValidationError> {
    let bytes = value.len();
    if bytes > MAX_DB_TABLE_TEXT_BYTES {
        return Err(DbTableValidationError::TextTooLarge {
            field,
            bytes,
            max: MAX_DB_TABLE_TEXT_BYTES,
        });
    }
    Ok(())
}
