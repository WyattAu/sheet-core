//! Spreadsheet data model — sparse cell storage, dependency DAG, topological
//! evaluation order, formula reference extraction.
//!
//! `sheet-core` is the estate's spreadsheet substrate: it models a worksheet
//! as a sparse `(row, col) → Cell` map and a worksheet's formulas as a
//! dependency graph with deterministic evaluation order. It knows nothing
//! about file formats — the XLSX codec lives in `sheet-xlsx` (L1, depends on
//! this crate).
//!
//! # Sparse storage
//!
//! A [`Sheet`] is a [`BTreeMap`] of coordinates to [`Cell`]s — absent
//! coordinates *are* the blank cells, so a million-row sheet with ten filled
//! cells costs ten entries. Coordinates are 0-based `(u32, u32)` internally;
//! [`refs`] converts to and from the `A1` notation users see.
//!
//! # Dependencies ([`DependencyGraph`])
//!
//! Formulas reference other cells (`=A1+B2`, `=SUM(A1:A9)`). The graph keeps
//! both directions — [`precedents`](DependencyGraph::precedents) (what a
//! formula reads) and [`dependents`](DependencyGraph::dependents) (what must
//! recompute when a cell changes) — and derives:
//!
//! - [`topological_order`](DependencyGraph::topological_order): a total,
//!   deterministic order in which every formula is evaluated after every
//!   formula it reads,
//! - [`dirty_subgraph`](DependencyGraph::dirty_subgraph): the minimal
//!   transitive recompute set for an edit, already in evaluation order,
//! - [`has_circular`](DependencyGraph::has_circular): the circular
//!   references, reported as explicit cycles instead of silently looping.
//!
//! # Reference extraction ([`refs`])
//!
//! [`DependencyGraph`] parses formulas with the built-in extractor in
//! [`refs`]: a lenient `A1`/`$A$1`/range/`Sheet!A1` scanner that walks the
//! formula once and collects cell references, skipping string literals,
//! function names, numbers, and everything it does not recognize. It is
//! deliberately *not* a full formula parser — it extracts exactly the edges
//! the dependency graph needs, and it is total: malformed input is a typed
//! [`GraphError`], never a panic.
//!
//! # Layer
//!
//! L1 substrate: **zero dependencies** (not even optional ones),
//! `no_std` + `alloc`, `#![forbid(unsafe_code)]`.
//!
//! # Example
//!
//! ```
//! use sheet_core::{Cell, CellValue, DependencyGraph, Sheet};
//!
//! # fn main() -> Result<(), sheet_core::GraphError> {
//! // Sparse storage: coordinates are 0-based, absence is blank.
//! let mut sheet = Sheet::new();
//! sheet.set(0, 0, Cell::number(41.0));
//! sheet.set(1, 0, Cell::formula("=A1+1"));
//! assert!(matches!(sheet.get(0, 0), Some(Cell { value: CellValue::Number(n), .. }) if *n == 41.0));
//!
//! // Dependencies: B2 reads A1, so A1's edit dirties B2.
//! let mut graph = DependencyGraph::new();
//! graph.add_formula((1, 0), "=A1+1")?;
//! assert_eq!(graph.precedents((1, 0)), vec![(0, 0)]);
//! assert_eq!(graph.dirty_subgraph(&[(0, 0)]), vec![(1, 0)]);
//! assert!(graph.has_circular().is_empty());
//! # Ok(())
//! # }
//! ```

#![no_std]
#![forbid(unsafe_code)]

extern crate alloc;

pub mod cell;
pub mod error;
pub mod graph;
pub mod refs;
pub mod sheet;

pub use cell::{Cell, CellValue, ExcelError};
pub use error::GraphError;
pub use graph::DependencyGraph;
pub use sheet::Sheet;
