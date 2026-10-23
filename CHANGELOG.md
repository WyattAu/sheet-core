# Changelog

All notable changes to this project are documented here. Format: [Keep a
Changelog](https://keepachangelog.com/): versions follow [semver](https://semver.org).

## [0.1.0] - 2026-10-04

### Added

- **Sparse cell storage**: `Sheet`: `BTreeMap<(row, col), Cell>` with
  0-based `u32` coordinates, `set`/`get`/`remove`, bounding-box
  `used_range`, row-major deterministic iteration (`iter` +
  `IntoIterator for &Sheet`), and range-bounded scans (`cells_in`).
  Absence is the blank: memory scales with filled cells.
- **Cell model**: `CellValue` (`Number`, `Text`, `Boolean`, `Error`,
  `Formula`, `Blank`), `Cell` with optional style index, and the XLSX
  error literals `ExcelError` (`#DIV/0!` … `#CALC!`) with lossless
  `Display`/`FromStr` round-tripping.
- **Dependency graph**: `DependencyGraph`: forward (formula →
  precedents) and reverse (cell → dependents) edges maintained on every
  `add_formula`/`remove_formula`; `precedents`, `dependents`,
  `topological_order` (deterministic Kahn), `dirty_subgraph` (minimal
  transitive recompute set, evaluation-ordered), and `has_circular`
  (explicit cycles via iterative Tarjan SCC, self-loops included).
- **Formula reference extraction**: `refs::extract_refs`: a lenient
  one-pass scanner for `A1`/`$A$1`/`Sheet!A1`/`'My Sheet'!A1` and
  rectangle ranges, skipping string literals, function names, numbers,
  and unknown identifiers. Typed `GraphError` for unterminated literals,
  dangling sheet qualifiers, `u32` coordinate overflow, and ranges above
  the 65,536-cell expansion cap (`RANGE_CELL_CAP`).
- **A1 notation conversions**: `cell_name`/`parse_cell_name`,
  `index_to_column`/`column_to_index`, shared with the estate's
  `sheet-xlsx` codec.

[0.1.0]: https://github.com/WyattAu/sheet-core/releases/tag/v0.1.0
