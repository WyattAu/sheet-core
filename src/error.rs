//! Typed, exhaustive error taxonomy for formula reference extraction.
//!
//! Following the estate's error discipline (see `wire-kit`): the enum is
//! deliberately **exhaustive** (no `#[non_exhaustive]`) — callers match every
//! variant, and a new variant is a semver-minor event by design. Extraction
//! is total: malformed formulas yield one of these variants, never a panic.

use alloc::string::String;
use core::fmt;

/// Everything that can go wrong while extracting references from a formula.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GraphError {
    /// The formula text was empty or held nothing but the leading `=`.
    EmptyFormula,
    /// A string literal (`"…"`) was opened but never closed. Bytes inside a
    /// string literal are not scanned for references, so an unterminated
    /// literal poisons the rest of the formula.
    UnterminatedString {
        /// Byte offset of the opening `"`.
        position: usize,
    },
    /// A quoted sheet name (`'My Sheet'!A1`) was opened but never closed.
    UnterminatedQuote {
        /// Byte offset of the opening `'`.
        position: usize,
    },
    /// An explicit sheet qualifier (`Sheet1!`, `'My Sheet'!`) was not
    /// followed by a parseable cell reference. Unlike unqualified junk —
    /// which the lenient scanner skips — a qualifier is explicit user
    /// intent, so its failure is reported.
    InvalidReference {
        /// Byte offset of the malformed reference.
        position: usize,
        /// The offending text (up to 32 bytes, truncated with `…`).
        text: String,
    },
    /// A column name would exceed the crate's coordinate space (`u32`
    /// column index, 0-based). Practical grids are far smaller; this guards
    /// pathological input like `AAAAAAAAAAAAAAAAAAAAAAAAAAAAAA1`.
    ColumnOverflow {
        /// Byte offset of the column name.
        position: usize,
        /// The offending column letters.
        text: String,
    },
    /// A row number would exceed the crate's coordinate space (`u32` row
    /// index, 0-based; row numbers are 1-based in `A1` notation).
    RowOverflow {
        /// Byte offset of the row digits.
        position: usize,
        /// The offending digits.
        text: String,
    },
    /// A range reference would expand to more than
    /// [`crate::refs::RANGE_CELL_CAP`] individual cells (e.g. a
    /// whole-column `A:A` reference). The per-cell dependency model
    /// requires materializing every cell in a range; refusing beats
    /// silently allocating millions of edges. Pass an explicit bounded
    /// range instead.
    RangeTooLarge {
        /// Normalized upper-left corner of the range (inclusive).
        start: (u32, u32),
        /// Normalized lower-right corner of the range (inclusive).
        end: (u32, u32),
        /// Number of cells the range covers.
        cells: u64,
    },
}

impl fmt::Display for GraphError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            GraphError::EmptyFormula => write!(f, "formula is empty"),
            GraphError::UnterminatedString { position } => {
                write!(f, "unterminated string literal at byte {position}")
            }
            GraphError::UnterminatedQuote { position } => {
                write!(f, "unterminated quoted sheet name at byte {position}")
            }
            GraphError::InvalidReference { position, text } => {
                write!(f, "invalid cell reference at byte {position}: {text:?}")
            }
            GraphError::ColumnOverflow { position, text } => {
                write!(
                    f,
                    "column {text:?} at byte {position} exceeds u32 coordinate space"
                )
            }
            GraphError::RowOverflow { position, text } => {
                write!(
                    f,
                    "row {text:?} at byte {position} exceeds u32 coordinate space"
                )
            }
            GraphError::RangeTooLarge { start, end, cells } => write!(
                f,
                "range ({},{}):({},{}) covers {cells} cells, above the expansion cap",
                start.0, start.1, end.0, end.1
            ),
        }
    }
}

impl core::error::Error for GraphError {}

#[cfg(test)]
mod tests {
    #![allow(
        clippy::unwrap_used,
        clippy::expect_used,
        clippy::panic,
        clippy::indexing_slicing
    )]

    use super::GraphError;
    use alloc::{format, string::String};

    #[test]
    fn display_is_informative_and_total() {
        let cases = [
            GraphError::EmptyFormula,
            GraphError::UnterminatedString { position: 3 },
            GraphError::UnterminatedQuote { position: 5 },
            GraphError::InvalidReference {
                position: 8,
                text: String::from("1A"),
            },
            GraphError::ColumnOverflow {
                position: 1,
                text: String::from("ZZZZ"),
            },
            GraphError::RowOverflow {
                position: 2,
                text: String::from("99999"),
            },
            GraphError::RangeTooLarge {
                start: (0, 0),
                end: (9, 9),
                cells: 100,
            },
        ];
        for e in cases {
            let s = format!("{e}");
            assert!(!s.is_empty(), "{e:?} rendered empty");
        }
    }
}
