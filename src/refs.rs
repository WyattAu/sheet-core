//! Formula reference extraction — the lenient `A1`-notation scanner.
//!
//! This module turns formula text into the set of cell coordinates a formula
//! reads. It is a **scanner, not a parser**: it walks the formula once,
//! pulls out everything shaped like a cell reference, and skips everything
//! else (string literals, function names, numbers, operators, unknown
//! identifiers). That leniency is the point — the dependency graph needs the
//! edges, and a full expression grammar (with function tables, operators,
//! array literals, macros) is a different crate's job.
//!
//! # What is recognized
//!
//! | Form | Example | Extracted |
//! |---|---|---|
//! | relative | `A1` | `(0, 0)` |
//! | column-absolute | `$A1` | `(0, 0)` |
//! | row-absolute | `A$1` | `(0, 0)` |
//! | fully absolute | `$A$1` | `(0, 0)` |
//! | rectangle range | `A1:B3` | all nine cells |
//! | sheet-qualified | `Sheet2!A1` | `(0, 0)` |
//! | quoted sheet | `'My Sheet'!$B$2` | `(1, 1)` |
//!
//! # What is deliberately skipped
//!
//! - **String literals** (`"sum in A1?"`) — quoted text is never scanned;
//!   `""` is the escape for an embedded quote.
//! - **Function names** — `LOG10(A1)` yields `A1`, never `LOG10` (the
//!   identifier directly followed by `(`).
//! - **Numbers** — the `E3` inside `1.5E+3` is not a cell.
//! - **Unrecognized identifiers** — `A1B2` (two refs jammed together) and
//!   named ranges are skipped, matching the scanner's lenient contract.
//!
//! # Sheet qualifiers and the per-sheet graph
//!
//! [`crate::DependencyGraph`] models **one worksheet**. A qualified
//! reference (`Sheet2!B3`) is parsed for syntax and recorded at its
//! coordinates `(2, 1)` regardless of which sheet named — a documented v0.1
//! simplification. Cross-sheet edges belong to a workbook-level graph; see
//! the README roadmap.
//!
//! # Coordinates
//!
//! Internally `(row, col)` is 0-based; `A1` notation is 1-based with
//! base-26 column letters. The conversions are public — `sheet-xlsx` reuses
//! them for the `r="B3"` attributes of the file format.

use crate::error::GraphError;
use alloc::string::String;
use alloc::vec::Vec;

/// Maximum number of individual cells a single range reference may expand
/// to. Whole-column (`A:A`, 1,048,576 cells) and whole-row (`1:1`, 16,384
/// cells) references above the cap are rejected with
/// [`GraphError::RangeTooLarge`] rather than materialized into millions of
/// graph edges.
pub const RANGE_CELL_CAP: u64 = 65_536;

/// Extracts every cell reference in `formula`, sorted and de-duplicated.
///
/// Accepts the text with or without the leading `=` (both `=A1+1` and
/// `A1+1` scan identically); only the *presence* of scannable content is
/// required. See the [module docs](self) for exactly what is recognized and
/// what is skipped.
///
/// # Errors
///
/// - [`GraphError::EmptyFormula`] when there is nothing to scan.
/// - [`GraphError::UnterminatedString`] / [`GraphError::UnterminatedQuote`]
///   when a literal or quoted sheet name is left open.
/// - [`GraphError::InvalidReference`] when an explicit sheet qualifier is
///   not followed by a valid reference.
/// - [`GraphError::ColumnOverflow`] / [`GraphError::RowOverflow`] when a
///   reference exceeds the `u32` coordinate space.
/// - [`GraphError::RangeTooLarge`] when a range expands beyond
///   [`RANGE_CELL_CAP`] cells.
pub fn extract_refs(formula: &str) -> Result<Vec<(u32, u32)>, GraphError> {
    let body = formula.trim();
    let body = body.strip_prefix('=').unwrap_or(body).trim();
    if body.is_empty() {
        return Err(GraphError::EmptyFormula);
    }

    let bytes = body.as_bytes();
    let mut refs = Vec::new();
    let mut i = 0usize;

    while let Some(&byte) = bytes.get(i) {
        match byte {
            b'"' => {
                // String literal: skip to the closing quote, `""` escapes.
                let open = i;
                i += 1;
                loop {
                    match bytes.get(i) {
                        None => return Err(GraphError::UnterminatedString { position: open }),
                        Some(b'"') => {
                            if bytes.get(i + 1) == Some(&b'"') {
                                i += 2; // escaped quote
                            } else {
                                i += 1; // end of literal
                                break;
                            }
                        }
                        Some(_) => i += 1,
                    }
                }
            }
            b'\'' => {
                // Quoted sheet name: '…' must be followed by `!` and a ref.
                let open = i;
                match skip_quoted_sheet(body, i) {
                    None => return Err(GraphError::UnterminatedQuote { position: open }),
                    Some(next) => {
                        if bytes.get(next) == Some(&b'!') {
                            i = parse_qualified_ref(body, next + 1, &mut refs)?;
                        } else {
                            // Quoted text with no qualifier is not a
                            // reference context; skip past it (lenient).
                            i = next;
                        }
                    }
                }
            }
            b'$' | b'A'..=b'Z' | b'a'..=b'z' => {
                if i > 0 && bytes.get(i - 1).copied().is_some_and(is_identifier_byte) {
                    // Mid-token (e.g. the `E3` in `1.5E3`): not a ref start.
                    i += 1;
                    continue;
                }
                i = scan_identifier(body, i, &mut refs)?;
            }
            _ => i += 1,
        }
    }

    refs.sort_unstable();
    refs.dedup();
    Ok(refs)
}

/// `true` when `b` can be part of an identifier-ish token we must not start
/// a reference inside of (letters, digits, `_`, `.`, `$`).
fn is_identifier_byte(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_' || b == b'.' || b == b'$'
}

/// Skips a quoted sheet name starting at `start` (which points at `'`).
/// Returns the offset just past the closing quote, or `None` if unterminated.
fn skip_quoted_sheet(s: &str, start: usize) -> Option<usize> {
    let bytes = s.as_bytes();
    let mut i = start + 1;
    loop {
        match bytes.get(i) {
            None => return None,
            Some(b'\'') => {
                if bytes.get(i + 1) == Some(&b'\'') {
                    i += 2; // escaped quote (`''`)
                } else {
                    return Some(i + 1);
                }
            }
            Some(_) => i += 1,
        }
    }
}

/// At `i`: parses an optional `Sheet!` qualifier, then optionally a
/// reference and a `:ref` range tail. Returns the new scan position.
///
/// The qualifier rule drives the error surface: an identifier followed by
/// `!` (or a quoted name — handled by the caller) *promises* a reference,
/// so a failure there is [`GraphError::InvalidReference`] rather than a
/// silent skip.
fn scan_identifier(s: &str, start: usize, refs: &mut Vec<(u32, u32)>) -> Result<usize, GraphError> {
    let bytes = s.as_bytes();
    let i = start;

    // Plain (unquoted) sheet name: identifier immediately followed by `!`.
    let mut j = i;
    while matches!(
        bytes.get(j),
        Some(&b) if b.is_ascii_alphanumeric() || b == b'_' || b == b'.'
    ) {
        j += 1;
    }
    if j > i && bytes.get(j) == Some(&b'!') {
        return parse_qualified_ref(s, j + 1, refs);
    }

    // Cell reference (possibly with `$`s), or a function name, or junk.
    match parse_a1(s, i)? {
        Some((coord, after)) => {
            if is_function_call(s, after) || bytes.get(after) == Some(&b'!') {
                // Function name (`LOG10(`) or sheet name (`A1!`): not a ref.
                return Ok(after);
            }
            refs.push(coord);
            Ok(parse_range_tail(s, after, coord, refs)?)
        }
        None => Ok(i + 1), // unknown identifier: skip one char, resync
    }
}

/// Parses the reference that must follow a `!` qualifier at `i`, plus an
/// optional range tail. Failure is an error, not a skip.
fn parse_qualified_ref(s: &str, i: usize, refs: &mut Vec<(u32, u32)>) -> Result<usize, GraphError> {
    let bytes = s.as_bytes();
    match bytes.get(i) {
        Some(&b) if b == b'$' || b.is_ascii_alphabetic() => match parse_a1(s, i)? {
            Some((coord, after)) => {
                if is_function_call(s, after) {
                    return Err(GraphError::InvalidReference {
                        position: i,
                        text: snippet(s, i),
                    });
                }
                refs.push(coord);
                Ok(parse_range_tail(s, after, coord, refs)?)
            }
            None => Err(GraphError::InvalidReference {
                position: i,
                text: snippet(s, i),
            }),
        },
        _ => Err(GraphError::InvalidReference {
            position: i,
            text: snippet(s, i),
        }),
    }
}

/// `true` when the token ending at `after` is immediately followed by `(` —
/// i.e. it is a function name, never a reference.
fn is_function_call(s: &str, after: usize) -> bool {
    s.as_bytes().get(after) == Some(&b'(')
}

/// After a first reference ending at `after`, consumes an optional
/// `whitespace ':' whitespace ref` range tail and expands the rectangle.
/// Returns the new scan position.
fn parse_range_tail(
    s: &str,
    after: usize,
    first: (u32, u32),
    refs: &mut Vec<(u32, u32)>,
) -> Result<usize, GraphError> {
    let bytes = s.as_bytes();
    let mut i = after;
    while matches!(bytes.get(i), Some(b' ' | b'\t')) {
        i += 1;
    }
    if bytes.get(i) != Some(&b':') {
        return Ok(after);
    }
    i += 1;
    while matches!(bytes.get(i), Some(b' ' | b'\t')) {
        i += 1;
    }
    match bytes.get(i) {
        Some(&b) if b == b'$' || b.is_ascii_alphabetic() => match parse_a1(s, i)? {
            Some((second, end)) => {
                expand_range(first, second, refs)?;
                Ok(end)
            }
            None => Ok(after), // lenient: keep the first ref, let the tail rescan
        },
        _ => Ok(after),
    }
}

/// Expands the rectangle between two corners (normalized) into per-cell
/// coordinates, enforcing [`RANGE_CELL_CAP`].
fn expand_range(
    a: (u32, u32),
    b: (u32, u32),
    refs: &mut Vec<(u32, u32)>,
) -> Result<(), GraphError> {
    let (r0, r1) = (a.0.min(b.0), a.0.max(b.0));
    let (c0, c1) = (a.1.min(b.1), a.1.max(b.1));
    let cells = u64::from(r1 - r0 + 1) * u64::from(c1 - c0 + 1);
    if cells > RANGE_CELL_CAP {
        return Err(GraphError::RangeTooLarge {
            start: (r0, c0),
            end: (r1, c1),
            cells,
        });
    }
    let mut r = r0;
    while r <= r1 {
        let mut c = c0;
        while c <= c1 {
            refs.push((r, c));
            c += 1;
        }
        r += 1;
    }
    Ok(())
}

/// One `A1` match: the 0-based `(row, col)` plus the offset just past it.
type A1Match = ((u32, u32), usize);

/// Parses one `A1`-style reference starting exactly at `start`.
///
/// Returns `Some` on a syntactic match, `None` when the token is not shaped
/// like a reference at all, and an error when it is shaped like one but
/// exceeds the coordinate space.
fn parse_a1(s: &str, start: usize) -> Result<Option<A1Match>, GraphError> {
    let bytes = s.as_bytes();
    let mut i = start;
    if bytes.get(i) == Some(&b'$') {
        i += 1;
    }

    let col_start = i;
    while matches!(bytes.get(i), Some(&b) if b.is_ascii_alphabetic()) {
        i += 1;
    }
    if i == col_start {
        return Ok(None); // no column letters: not a reference
    }
    let col_letters = s.get(col_start..i).unwrap_or_default(); // ASCII, always valid

    if bytes.get(i) == Some(&b'$') {
        i += 1;
    }

    let row_start = i;
    while matches!(bytes.get(i), Some(&b) if b.is_ascii_digit()) {
        i += 1;
    }
    if i == row_start {
        return Ok(None); // letters but no digits: an identifier, not a ref
    }
    let row_digits = s.get(row_start..i).unwrap_or_default();

    // Boundary: the token must end at a non-identifier char (`A1B2` is one
    // jammed token, not a reference followed by an identifier).
    if matches!(bytes.get(i), Some(&b) if is_identifier_byte(b)) {
        return Ok(None);
    }

    let col = parse_column(col_letters, col_start)?;
    let row = parse_row(row_digits, row_start)?;
    Ok(Some(((row, col), i)))
}

/// Base-26 bijective column letters → 0-based index (`A` → 0, `Z` → 25,
/// `AA` → 26). Empty input is rejected (it names no column).
fn parse_column(letters: &str, position: usize) -> Result<u32, GraphError> {
    if letters.is_empty() {
        return Err(GraphError::InvalidReference {
            position,
            text: alloc::string::String::from(letters),
        });
    }
    let mut col: u64 = 0;
    for b in letters.bytes() {
        col = col
            .checked_mul(26)
            .and_then(|c| c.checked_add(u64::from(b.to_ascii_uppercase() - b'A' + 1)))
            .ok_or(GraphError::ColumnOverflow {
                position,
                text: alloc::string::String::from(letters),
            })?;
    }
    let zero_based = col - 1; // bijective numeration: total value is 1-based
    u32::try_from(zero_based).map_err(|_| GraphError::ColumnOverflow {
        position,
        text: alloc::string::String::from(letters),
    })
}

/// 1-based row digits → 0-based row index (`1` → 0).
fn parse_row(digits: &str, position: usize) -> Result<u32, GraphError> {
    let mut row: u64 = 0;
    for b in digits.bytes() {
        row = row
            .checked_mul(10)
            .and_then(|r| r.checked_add(u64::from(b - b'0')))
            .ok_or(GraphError::RowOverflow {
                position,
                text: alloc::string::String::from(digits),
            })?;
    }
    if row == 0 {
        return Ok(0); // `A0` degenerates to row 0 — the lenient read
    }
    u32::try_from(row - 1).map_err(|_| GraphError::RowOverflow {
        position,
        text: alloc::string::String::from(digits),
    })
}

/// A bounded text snippet for error messages (up to 32 bytes).
fn snippet(s: &str, from: usize) -> String {
    let end = s.len().min(from + 32);
    match s.get(from..end) {
        Some(text) => String::from(text),
        None => String::from("…"),
    }
}

/// 0-based column index → base-26 letters (`0` → `A`, `25` → `Z`,
/// `26` → `AA`). Inverse of [`parse_column`].
#[must_use]
pub fn index_to_column(mut index: u32) -> String {
    let mut digits: Vec<u8> = Vec::with_capacity(7);
    loop {
        digits.push(b'A' + (index % 26) as u8);
        index /= 26;
        if index == 0 {
            break;
        }
        index -= 1;
    }
    digits.reverse();
    digits.into_iter().map(|b| b as char).collect()
}

/// Base-26 bijective column letters → 0-based index (`A` → 0). Strict:
/// uppercase or lowercase ASCII letters only, and within the `u32`
/// coordinate space. Inverse of [`index_to_column`].
///
/// # Errors
///
/// [`GraphError::InvalidReference`] for non-alphabetic input,
/// [`GraphError::ColumnOverflow`] beyond the `u32` coordinate space.
pub fn column_to_index(letters: &str) -> Result<u32, GraphError> {
    if !letters.bytes().all(|b| b.is_ascii_alphabetic()) {
        return Err(GraphError::InvalidReference {
            position: 0,
            text: alloc::string::String::from(letters),
        });
    }
    parse_column(letters, 0)
}

/// `(row, col)` → `A1` notation, 1-based (`(0, 0)` → `"A1"`).
#[must_use]
pub fn cell_name(coord: (u32, u32)) -> String {
    let mut name = index_to_column(coord.1);
    name.push_str(alloc::format!("{}", coord.0 + 1).as_str());
    name
}

/// `A1` notation → `(row, col)`, 0-based (`"B3"` → `(2, 1)`). Strict: the
/// whole string must be one reference. Inverse of [`cell_name`].
///
/// # Errors
///
/// [`GraphError::InvalidReference`] unless the entire input is a valid
/// reference within the `u32` coordinate space.
pub fn parse_cell_name(name: &str) -> Result<(u32, u32), GraphError> {
    let trimmed = name.trim();
    match parse_a1(trimmed, 0)? {
        Some((coord, after)) if after == trimmed.len() => Ok(coord),
        _ => Err(GraphError::InvalidReference {
            position: 0,
            text: alloc::string::String::from(name),
        }),
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

    use super::{
        cell_name, column_to_index, extract_refs, index_to_column, parse_cell_name, RANGE_CELL_CAP,
    };
    use crate::error::GraphError;
    use alloc::{format, string::String, vec, vec::Vec};

    fn refs(formula: &str) -> Vec<(u32, u32)> {
        // Test-only: assertions legitimately unwrap; the lib paths stay
        // lint-clean.
        extract_refs(formula).unwrap_or_else(|e| panic!("test formula {formula:?} failed: {e}"))
    }

    #[test]
    fn plain_relative_and_absolute_forms() {
        assert_eq!(refs("=A1"), vec![(0, 0)]);
        assert_eq!(refs("=$A$1"), vec![(0, 0)]);
        assert_eq!(refs("=$A1"), vec![(0, 0)]);
        assert_eq!(refs("=A$1"), vec![(0, 0)]);
        assert_eq!(refs("B2"), vec![(1, 1)]);
        assert_eq!(refs("=a1+b2"), vec![(0, 0), (1, 1)]);
        // Leading `=` optional; whitespace trimmed.
        assert_eq!(refs("  =C3  "), vec![(2, 2)]);
    }

    #[test]
    fn functions_strings_and_numbers_are_not_references() {
        assert_eq!(refs("=LOG10(A1)"), vec![(0, 0)]); // LOG10 is a function
        assert_eq!(
            refs("=SUM(A1:B2)+MAX(C1,C2)"),
            vec![(0, 0), (0, 1), (0, 2), (1, 0), (1, 1), (1, 2)]
        );
        assert_eq!(refs("=\"A1 is quoted\"&A2"), vec![(1, 0)]);
        assert_eq!(refs("=\"say \"\"A1\"\" ok\""), Vec::<(u32, u32)>::new());
        assert_eq!(refs("=1.5E3+0.5"), Vec::<(u32, u32)>::new()); // E3 is part of the number
        assert_eq!(refs("=IF(A1>3,TRUE,FALSE)"), vec![(0, 0)]); // TRUE/FALSE skipped
        assert_eq!(refs("=1A+2B"), Vec::<(u32, u32)>::new()); // jammed junk skipped
    }

    #[test]
    fn ranges_expand_per_cell_and_normalize() {
        assert_eq!(refs("=A1:B2"), vec![(0, 0), (0, 1), (1, 0), (1, 1)]);
        assert_eq!(refs("=B2:A1"), vec![(0, 0), (0, 1), (1, 0), (1, 1)]); // normalized
        assert_eq!(refs("=$B$2 : $C$3"), vec![(1, 1), (1, 2), (2, 1), (2, 2)]);
        assert_eq!(
            refs("=SUM($A$1:$B$2, C3)"),
            vec![(0, 0), (0, 1), (1, 0), (1, 1), (2, 2)]
        );
    }

    #[test]
    fn sheet_qualifiers_parse_including_quoted_names() {
        assert_eq!(refs("=Sheet2!B3"), vec![(2, 1)]);
        assert_eq!(refs("='My Sheet'!$B$2"), vec![(1, 1)]);
        assert_eq!(refs("='It''s'!A1:A2"), vec![(0, 0), (1, 0)]); // '' escape
        assert_eq!(
            refs("=SUM(Sheet2!A1:B2)"),
            vec![(0, 0), (0, 1), (1, 0), (1, 1)]
        );
        // Qualified refs to another sheet still record coordinates (per-sheet
        // model, documented simplification).
        assert_eq!(refs("=Sheet1!A1+1"), vec![(0, 0)]);
    }

    #[test]
    fn error_cases() {
        assert_eq!(extract_refs(""), Err(GraphError::EmptyFormula));
        assert_eq!(extract_refs("   "), Err(GraphError::EmptyFormula));
        assert_eq!(extract_refs("="), Err(GraphError::EmptyFormula));
        assert_eq!(
            extract_refs("=\"abc"),
            Err(GraphError::UnterminatedString { position: 0 })
        );
        assert_eq!(
            extract_refs("='Sheet!A1"),
            Err(GraphError::UnterminatedQuote { position: 0 })
        );
        assert_eq!(
            extract_refs("=Sheet1!"),
            Err(GraphError::InvalidReference {
                position: 7,
                text: String::new()
            })
        );
        assert_eq!(
            extract_refs("=Sheet1!1A"),
            Err(GraphError::InvalidReference {
                position: 7,
                text: String::from("1A")
            })
        );
        assert_eq!(
            extract_refs("=Sheet1!Q+1"),
            Err(GraphError::InvalidReference {
                position: 7,
                text: String::from("Q+1")
            })
        );
    }

    #[test]
    fn coordinate_overflow_is_typed() {
        let wide = format!("={}1", "A".repeat(40));
        assert!(matches!(
            extract_refs(&wide),
            Err(GraphError::ColumnOverflow { position: 0, .. })
        ));
        let tall = format!("=A{}", u64::from(u32::MAX) + 2);
        assert!(matches!(
            extract_refs(&tall),
            Err(GraphError::RowOverflow { position: 1, .. })
        ));
    }

    #[test]
    fn ranges_above_the_cap_are_rejected() {
        // Two full columns exceed the 65_536-cell cap.
        assert_eq!(
            extract_refs("=A1:B1048576"),
            Err(GraphError::RangeTooLarge {
                start: (0, 0),
                end: (1_048_575, 1),
                cells: 2_097_152,
            })
        );
        // Exactly under the cap is fine (4096 rows × 2 cols = 8192 cells).
        let under_cap =
            extract_refs("=A1:B4096").unwrap_or_else(|e| panic!("sub-cap range failed: {e}"));
        assert_eq!(under_cap.len(), 8192);
        // 256 rows × 256 cols = 65_536 == the cap: still accepted.
        assert_eq!(
            extract_refs("=A1:IV256").map(|r| r.len() as u64),
            Ok(RANGE_CELL_CAP)
        );
    }

    #[test]
    fn a1_notation_conversion_round_trips() {
        assert_eq!(index_to_column(0), "A");
        assert_eq!(index_to_column(25), "Z");
        assert_eq!(index_to_column(26), "AA");
        assert_eq!(index_to_column(27), "AB");
        assert_eq!(index_to_column(701), "ZZ");
        assert_eq!(index_to_column(702), "AAA");
        assert_eq!(column_to_index("A"), Ok(0));
        assert_eq!(column_to_index("ZZ"), Ok(701));
        assert_eq!(column_to_index("aaa"), Ok(702)); // case-insensitive
        assert!(column_to_index("A1").is_err());
        assert!(column_to_index("").is_err()); // empty names no column

        for col in [0u32, 1, 25, 26, 700, 16_383, 100_000] {
            assert_eq!(column_to_index(&index_to_column(col)), Ok(col), "col {col}");
        }
        assert_eq!(cell_name((0, 0)), "A1");
        assert_eq!(cell_name((1_048_575, 16_383)), "XFD1048576");
        assert_eq!(parse_cell_name("XFD1048576"), Ok((1_048_575, 16_383)));
        assert_eq!(parse_cell_name(" b3 "), Ok((2, 1)));
        assert!(parse_cell_name("3B").is_err());
        assert!(parse_cell_name("A1B2").is_err());
        assert!(parse_cell_name("$A$1").is_ok()); // absolute form accepted
    }

    #[test]
    fn formula_text_needs_not_start_with_equals() {
        assert_eq!(refs("A1+A2"), vec![(0, 0), (1, 0)]);
        assert_eq!(extract_refs("="), Err(GraphError::EmptyFormula));
    }
}
