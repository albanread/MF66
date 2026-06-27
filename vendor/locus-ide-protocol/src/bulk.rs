use serde::{Deserialize, Serialize};

use crate::ids::BulkId;
use crate::trust::TrustLabel;

pub const MAX_BULK_BYTES: u64 = 512 * 1024 * 1024;

#[derive(Clone, Copy, Debug, Eq, PartialEq, Deserialize, Serialize)]
pub enum BulkKind {
    PresentPixels,
    TablePage,
    TextBlob,
    PluginBlob,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Deserialize, Serialize)]
pub enum BulkAccess {
    ReadOnly,
    WriteOnly,
    ReadWrite,
}

#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
pub struct BulkDescriptor {
    pub bulk_id: BulkId,
    pub byte_len: u64,
    pub kind: BulkKind,
    pub access: BulkAccess,
    pub generation: u64,
    pub trust: TrustLabel,
    pub transport: BulkTransport,
}

#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
pub enum BulkTransport {
    SharedMemoryName {
        name: String,
        byte_offset: u64,
        mapped_len: u64,
    },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BulkValidationError {
    ZeroId,
    Empty,
    TooLarge {
        byte_len: u64,
        max: u64,
    },
    ZeroGeneration,
    WrongKind {
        found: BulkKind,
        expected: BulkKind,
    },
    WrongAccess {
        found: BulkAccess,
        expected: BulkAccess,
    },
    EmptyTransportName,
    EmptyTransport,
    TransportTooLarge {
        mapped_len: u64,
        max: u64,
    },
    TransportRangeOverflow {
        byte_offset: u64,
        byte_len: u64,
    },
    TransportTooSmall {
        byte_offset: u64,
        byte_len: u64,
        mapped_len: u64,
    },
}

impl BulkDescriptor {
    pub fn validate(&self) -> Result<(), BulkValidationError> {
        if self.bulk_id.is_zero() {
            return Err(BulkValidationError::ZeroId);
        }
        if self.byte_len == 0 {
            return Err(BulkValidationError::Empty);
        }
        if self.byte_len > MAX_BULK_BYTES {
            return Err(BulkValidationError::TooLarge {
                byte_len: self.byte_len,
                max: MAX_BULK_BYTES,
            });
        }
        if self.generation == 0 {
            return Err(BulkValidationError::ZeroGeneration);
        }
        self.transport.validate_for_payload(self.byte_len)?;
        Ok(())
    }

    pub fn validate_for(
        &self,
        expected_kind: BulkKind,
        expected_access: BulkAccess,
    ) -> Result<(), BulkValidationError> {
        self.validate()?;
        if self.kind != expected_kind {
            return Err(BulkValidationError::WrongKind {
                found: self.kind,
                expected: expected_kind,
            });
        }
        if self.access != expected_access {
            return Err(BulkValidationError::WrongAccess {
                found: self.access,
                expected: expected_access,
            });
        }
        Ok(())
    }
}

impl BulkTransport {
    pub fn shared_memory_name(name: impl Into<String>, byte_offset: u64, mapped_len: u64) -> Self {
        Self::SharedMemoryName {
            name: name.into(),
            byte_offset,
            mapped_len,
        }
    }

    fn validate_for_payload(&self, byte_len: u64) -> Result<(), BulkValidationError> {
        match self {
            Self::SharedMemoryName {
                name,
                byte_offset,
                mapped_len,
            } => {
                if name.is_empty() {
                    return Err(BulkValidationError::EmptyTransportName);
                }
                if *mapped_len == 0 {
                    return Err(BulkValidationError::EmptyTransport);
                }
                if *mapped_len > MAX_BULK_BYTES {
                    return Err(BulkValidationError::TransportTooLarge {
                        mapped_len: *mapped_len,
                        max: MAX_BULK_BYTES,
                    });
                }
                let Some(end) = byte_offset.checked_add(byte_len) else {
                    return Err(BulkValidationError::TransportRangeOverflow {
                        byte_offset: *byte_offset,
                        byte_len,
                    });
                };
                if end > *mapped_len {
                    return Err(BulkValidationError::TransportTooSmall {
                        byte_offset: *byte_offset,
                        byte_len,
                        mapped_len: *mapped_len,
                    });
                }
                Ok(())
            }
        }
    }
}
