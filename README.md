# sheet-core
[![Estate integration](https://github.com/WyattAu/estate-integration/actions/workflows/ci.yml/badge.svg)](https://github.com/WyattAu/estate-integration/actions/workflows/ci.yml)

Spreadsheet data model: sparse cell storage, dependency DAG, topological
evaluation order, formula reference extraction.

`sheet-core` is the estate's spreadsheet substrate: a worksheet is a sparse
`(row, col) → Cell` map (absent coordinates *are* the blank cells), and a
worksheet's formulas form a dependency graph with a deterministic evaluation
order and incremental recompute sets. It knows nothing about file formats;
the XLSX codec is [`sheet-xlsx`](https://github.com/WyattAu/sheet-xlsx).

## Design

- **Sparse by default.** `Sheet` is a `BTreeMap` of coordinates; a
  million-row sheet with ten filled cells costs ten entries. Iteration is
  row-major and deterministic.
- **Two directions, always consistent.** `DependencyGraph` keeps formula →
  precedents and cell → dependents in lockstep on every insert and remove.
  `d ∈ dependents(c)` ⟺ `c ∈ precedents(d)` holds by construction and is
  property-tested on random graphs.
- **Determinism.** `topological_order` (Kahn with a sorted ready set),
  `dirty_subgraph` (reverse BFS + internal ordering), and `has_circular`
  (iterative Tarjan SCC, ascending neighbors) are pure functions of the
  edge set: same edges in, same order out, everywhere.
- **Cycles are represented, not rejected.** Real workbooks contain them.
  `has_circular` reports the exact cycles (self-references included);
  `topological_order` omits cycle members *and everything that reads them*
  (they can never evaluate); the rest of the graph still orders.
- **Lenient reference extraction.** `refs::extract_refs` walks a formula
  once and pulls out everything shaped like `A1`, `$A$1`, `Sheet2!B3`,
  `'My Sheet'!C4`, or a rectangle range: skipping string literals,
  function names (`LOG10(...)`), numbers (`1.5E+3`), and unrecognized
  identifiers. Malformed-but-explicit constructs (a dangling `Sheet1!`,
  unterminated literals, `u32` coordinate overflow, ranges above the
  65,536-cell cap) fail with a typed `GraphError`, never a panic.

## Single-sheet scope

The graph models **one worksheet**. A `Sheet2!B3` qualifier inside a
formula on Sheet1 is parsed for syntax and recorded at `(2, 1)`: the
per-sheet coordinate space. Workbook-level cross-sheet edges belong in a
workbook graph layered on top (roadmap; `sheet-xlsx` is the natural home).

## Example

```rust
use sheet_core::{Cell, CellValue, DependencyGraph, Sheet};

# fn main() -> Result<(), sheet_core::GraphError> {
let mut sheet = Sheet::new();
sheet.set(0, 0, Cell::number(41.0));        // A1
sheet.set(0, 1, Cell::number(1.0));         // B1
sheet.set(1, 0, Cell::formula("=A1+B1"));   // A2
sheet.set(2, 0, Cell::formula("=A2*2"));    // A3

let mut graph = DependencyGraph::new();
for (coord, cell) in &sheet {
    if let CellValue::Formula(text) = &cell.value {
        graph.add_formula(coord, text)?;
    }
}

assert_eq!(graph.topological_order(), vec![(1, 0), (2, 0)]);
assert_eq!(graph.dirty_subgraph(&[(0, 0)]), vec![(1, 0), (2, 0)]);
assert!(graph.has_circular().is_empty());
# Ok(())
# }
```

## Layer

L1 substrate (see `docs/layers.md` in WyattAu/engineering-standards):
zero dependencies, `no_std` + `alloc`, `#![forbid(unsafe_code)]`, deny-on-
unwrap/expect/panic/indexing lints.

## Gates

`cargo build --locked --all-features` and `--no-default-features` ·
`cargo test --all-features` · `clippy -D warnings` (+ pedantic sweep) ·
`cargo fmt --check` · `cargo deny check` · `cargo vet` · coverage ≥ 90%
(currently ~97%) · estate layer check: enforced by the shared
[rust-kit workflow](https://github.com/WyattAu/engineering-standards).

## License

MIT OR Apache-2.0.
