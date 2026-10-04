//! Cell model — values, style indices, and the XLSX error literals.
//!
//! A [`Cell`] is a [`CellValue`] plus an optional style index. Values cover
//! everything a spreadsheet grid can hold: numbers, text, booleans, the
//! eight-plus XLSX error literals ([`ExcelError`]), formula text, and the
//! explicit blank.
//!
//! Coordinates live one level up ([`crate::Sheet`]): a cell never knows its
//! own address, so cells are freely movable and the sparse map stays the
//! single source of truth for occupancy.

use core::fmt;
use core::str::FromStr;

/// The XLSX error literals — what a cell evaluates to when a formula fails.
///
/// The `Display` implementation renders the canonical spreadsheet spelling
/// (`#DIV/0!`, …) and [`FromStr`] parses that spelling back, so the type
/// round-trips through the text-oriented file formats losslessly.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ExcelError {
    /// `#DIV/0!` — division by zero (or by an empty cell).
    Div0,
    /// `#N/A` — a lookup found nothing (`NA()`, failed `VLOOKUP`).
    Na,
    /// `#NAME?` — an unknown function or named range.
    Name,
    /// `#NULL!` — a range intersection operator found no intersection.
    Null,
    /// `#NUM!` — a numeric argument or result is out of domain
    /// (`SQRT(-1)`, overflow).
    Num,
    /// `#REF!` — a reference points at a deleted or invalid cell.
    Ref,
    /// `#VALUE!` — an argument has the wrong type (text where a number
    /// belongs).
    Value,
    /// `#SPILL!` — a dynamic-array result is blocked by occupied cells.
    Spill,
    /// `#CALC!` — a calc-engine failure with no more specific literal.
    Calc,
}

impl ExcelError {
    /// The canonical XLSX spelling of this error, e.g. `#DIV/0!`.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            ExcelError::Div0 => "#DIV/0!",
            ExcelError::Na => "#N/A",
            ExcelError::Name => "#NAME?",
            ExcelError::Null => "#NULL!",
            ExcelError::Num => "#NUM!",
            ExcelError::Ref => "#REF!",
            ExcelError::Value => "#VALUE!",
            ExcelError::Spill => "#SPILL!",
            ExcelError::Calc => "#CALC!",
        }
    }
}

impl fmt::Display for ExcelError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for ExcelError {
    type Err = ();

    /// Parses a canonical XLSX error literal (`#REF!`) into an
    /// [`ExcelError`]. Matching is exact and case-sensitive, mirroring the
    /// file format. Unknown literals parse to `Err(())`.
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "#DIV/0!" => Ok(ExcelError::Div0),
            "#N/A" => Ok(ExcelError::Na),
            "#NAME?" => Ok(ExcelError::Name),
            "#NULL!" => Ok(ExcelError::Null),
            "#NUM!" => Ok(ExcelError::Num),
            "#REF!" => Ok(ExcelError::Ref),
            "#VALUE!" => Ok(ExcelError::Value),
            "#SPILL!" => Ok(ExcelError::Spill),
            "#CALC!" => Ok(ExcelError::Calc),
            _ => Err(()),
        }
    }
}

/// Everything a grid cell can hold.
///
/// `Formula` carries the formula **text** verbatim as written (typically
/// with its leading `=`) — `sheet-core` does not evaluate formulas. The
/// dependency graph over formula text lives in
/// [`crate::DependencyGraph`].
#[derive(Debug, Clone, PartialEq)]
pub enum CellValue {
    /// A floating-point number — the universal numeric storage of every
    /// mainstream spreadsheet (dates included: they are numbers plus style).
    Number(f64),
    /// Text. Empty text is distinct from [`CellValue::Blank`]: the cell
    /// exists and holds a zero-length string.
    Text(alloc::string::String),
    /// A boolean.
    Boolean(bool),
    /// A formula error result.
    Error(ExcelError),
    /// Formula text, stored verbatim (usually including the leading `=`).
    /// A cell whose value is a formula has no cached result in this model;
    /// engines that cache results layer that on top.
    Formula(alloc::string::String),
    /// The explicit blank: the cell exists (and may carry a style) but holds
    /// no value. Absence from the sheet map is the *implicit* blank.
    Blank,
}

impl CellValue {
    /// Convenience constructor: `CellValue::Number(n)`.
    #[must_use]
    pub const fn number(n: f64) -> Self {
        CellValue::Number(n)
    }

    /// Convenience constructor: `CellValue::Text(s)`.
    #[must_use]
    pub fn text(s: impl Into<alloc::string::String>) -> Self {
        CellValue::Text(s.into())
    }

    /// Convenience constructor: `CellValue::Boolean(b)`.
    #[must_use]
    pub const fn boolean(b: bool) -> Self {
        CellValue::Boolean(b)
    }

    /// Convenience constructor: `CellValue::Error(e)`.
    #[must_use]
    pub const fn error(e: ExcelError) -> Self {
        CellValue::Error(e)
    }

    /// Convenience constructor: `CellValue::Formula(text)`.
    #[must_use]
    pub fn formula(text: impl Into<alloc::string::String>) -> Self {
        CellValue::Formula(text.into())
    }

    /// `true` when this value is a [`CellValue::Formula`].
    #[must_use]
    pub const fn is_formula(&self) -> bool {
        matches!(self, CellValue::Formula(_))
    }
}

/// One grid cell: a value and an optional style index.
///
/// `style` is an index into the workbook's style table (the `cellXfs` list
/// in XLSX terms). `sheet-core` stores only the index — style *definitions*
/// (fonts, fills, number formats) belong to the format codec.
#[derive(Debug, Clone, PartialEq)]
pub struct Cell {
    /// The cell's value.
    pub value: CellValue,
    /// Index into the workbook's style table, if the cell is styled.
    pub style: Option<u32>,
}

impl Cell {
    /// A cell holding `value` with no style.
    #[must_use]
    pub const fn new(value: CellValue) -> Self {
        Cell { value, style: None }
    }

    /// Convenience constructor: a number cell.
    #[must_use]
    pub const fn number(n: f64) -> Self {
        Cell::new(CellValue::Number(n))
    }

    /// Convenience constructor: a text cell.
    #[must_use]
    pub fn text(s: impl Into<alloc::string::String>) -> Self {
        Cell::new(CellValue::Text(s.into()))
    }

    /// Convenience constructor: a boolean cell.
    #[must_use]
    pub const fn boolean(b: bool) -> Self {
        Cell::new(CellValue::Boolean(b))
    }

    /// Convenience constructor: a formula-error cell.
    #[must_use]
    pub const fn error(e: ExcelError) -> Self {
        Cell::new(CellValue::Error(e))
    }

    /// Convenience constructor: a formula cell.
    #[must_use]
    pub fn formula(text: impl Into<alloc::string::String>) -> Self {
        Cell::new(CellValue::Formula(text.into()))
    }

    /// Convenience constructor: the explicit blank.
    #[must_use]
    pub const fn blank() -> Self {
        Cell::new(CellValue::Blank)
    }

    /// Builder: attaches style index `style`.
    #[must_use]
    pub const fn with_style(mut self, style: u32) -> Self {
        self.style = Some(style);
        self
    }
}

#[cfg(test)]
mod tests {
    #![allow(
        clippy::unwrap_used,
        clippy::expect_used,
        clippy::panic,
        clippy::indexing_slicing
    )]

    use super::{Cell, CellValue, ExcelError};
    use alloc::format;
    use alloc::string::ToString;
    use core::str::FromStr;

    #[test]
    fn error_literals_round_trip_through_display_and_from_str() {
        let all = [
            ExcelError::Div0,
            ExcelError::Na,
            ExcelError::Name,
            ExcelError::Null,
            ExcelError::Num,
            ExcelError::Ref,
            ExcelError::Value,
            ExcelError::Spill,
            ExcelError::Calc,
        ];
        for e in all {
            let rendered = format!("{e}");
            assert_eq!(rendered, e.as_str(), "{e:?} Display/as_str mismatch");
            assert_eq!(ExcelError::from_str(&rendered), Ok(e), "{e:?} round trip");
        }
    }

    #[test]
    fn unknown_error_literal_is_rejected() {
        assert_eq!(ExcelError::from_str("#NOPE!"), Err(()));
        assert_eq!(ExcelError::from_str("#ref!"), Err(())); // case-sensitive
        assert_eq!(ExcelError::from_str(""), Err(()));
    }

    #[test]
    fn cell_constructors_and_style_builder() {
        let styled = Cell::number(1.5).with_style(7);
        assert_eq!(
            styled,
            Cell {
                value: CellValue::Number(1.5),
                style: Some(7)
            }
        );
        assert_eq!(Cell::text("hi").style, None);
        assert_eq!(Cell::boolean(true).value, CellValue::Boolean(true));
        assert_eq!(
            Cell::formula("=A1").value,
            CellValue::Formula("=A1".to_string())
        );
        assert_eq!(Cell::blank().value, CellValue::Blank);
        assert_eq!(Cell::new(CellValue::Blank).style, None);
    }

    #[test]
    fn value_helpers() {
        assert!(CellValue::formula("=A1").is_formula());
        assert!(!CellValue::number(0.0).is_formula());
        assert!(!CellValue::Blank.is_formula());
        assert_eq!(CellValue::text("x"), CellValue::Text("x".to_string()));
        assert_eq!(CellValue::boolean(false), CellValue::Boolean(false));
        assert_eq!(CellValue::number(2.0), CellValue::Number(2.0));
    }
}
