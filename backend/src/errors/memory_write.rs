//! Closed, content-free diagnostics for native memory writes. Never retain the
//! rejected text, a secret match, or an underlying database error here.
use super::AppError;
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MemoryWriteReason {
    NoteTooLong,
    NoteEmpty,
    SecretDetected,
    InvalidNote,
    MemoryFull,
    ReplaceTargetNotFound,
    MemoryConflict,
    MemoryUnauthorized,
    MemoryStorageError,
    MemoryInternalError,
}

impl MemoryWriteReason {
    pub fn category(self) -> &'static str {
        match self {
            Self::NoteTooLong | Self::NoteEmpty | Self::SecretDetected | Self::InvalidNote => {
                "validation"
            }
            Self::MemoryFull | Self::ReplaceTargetNotFound | Self::MemoryConflict => "conflict",
            Self::MemoryUnauthorized => "authorization",
            Self::MemoryStorageError => "storage",
            Self::MemoryInternalError => "internal",
        }
    }

    pub fn message(self) -> &'static str {
        match self {
            Self::NoteTooLong => {
                "The note exceeds 500 characters. Shorten it or split it into separate facts of at most 500 characters each."
            }
            Self::NoteEmpty => {
                "The note is empty. Supply a fact containing at least one non-whitespace character."
            }
            Self::SecretDetected => {
                "The note resembles a credential or secret. Remove credentials and secret values; never save them in memory."
            }
            Self::InvalidNote => {
                "Invalid memory arguments. Supply text of 1 to 500 characters and, when replacing, a current note ID."
            }
            Self::MemoryFull => {
                "Memory holds at most 50 notes. Inspect the notes, then replace or forget one before saving another."
            }
            Self::ReplaceTargetNotFound => {
                "The note to replace no longer exists. Inspect current notes and choose an existing note ID, or save a new note."
            }
            Self::MemoryConflict => {
                "Memory or the agent changed before this write could apply. Inspect the agent and current notes before retrying."
            }
            Self::MemoryUnauthorized => {
                "Memory is unavailable in this context or you lack permission to edit it. Use a current agent ID in an authorized private thread; do not retry against another account."
            }
            Self::MemoryStorageError => {
                "Memory storage could not confirm the write. Inspect current notes before retrying; do not claim the fact was saved."
            }
            Self::MemoryInternalError => {
                "NyxID could not confirm the memory write. Inspect current notes before retrying; do not claim the fact was saved."
            }
        }
    }

    /// Preserve the existing HTTP/numeric error classes; the reason is additive.
    pub fn legacy_error(self) -> AppError {
        let message = self.message().into();
        match self {
            Self::MemoryFull | Self::MemoryConflict => AppError::Conflict(message),
            Self::ReplaceTargetNotFound => AppError::NotFound(message),
            Self::MemoryUnauthorized => AppError::Forbidden(message),
            Self::MemoryStorageError | Self::MemoryInternalError => AppError::Internal(message),
            _ => AppError::ValidationError(message),
        }
    }

    pub fn error_key(self) -> &'static str {
        match self {
            Self::MemoryFull | Self::MemoryConflict => "conflict",
            Self::ReplaceTargetNotFound => "not_found",
            Self::MemoryUnauthorized => "forbidden",
            Self::MemoryStorageError => "database_error",
            Self::MemoryInternalError => "internal_error",
            _ => "validation_error",
        }
    }

    pub fn from_error(error: &AppError) -> Self {
        match error {
            AppError::MemoryWrite(reason) => *reason,
            AppError::DatabaseError(error)
                if error.contains_label("TransientTransactionError")
                    && !error.contains_label("UnknownTransactionCommitResult") =>
            {
                Self::MemoryConflict
            }
            AppError::DatabaseError(_) => Self::MemoryStorageError,
            AppError::Conflict(_) => Self::MemoryConflict,
            AppError::NotFound(_)
            | AppError::Forbidden(_)
            | AppError::ForbiddenWithGuidance { .. }
            | AppError::Unauthorized(_)
            | AppError::AssistantMissingAccess(_) => Self::MemoryUnauthorized,
            AppError::ValidationError(_) | AppError::BadRequest(_) => Self::InvalidNote,
            _ => Self::MemoryInternalError,
        }
    }
}

impl std::fmt::Display for MemoryWriteReason {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.message())
    }
}
