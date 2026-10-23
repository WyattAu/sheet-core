# Security Policy: sheet-core

## Supported versions

| Version | Supported |
|---------|-----------|
| 0.1.x   | ✅        |

## Reporting a vulnerability

Report privately via [GitHub security advisories] for this repository, or
email **wyatt_au@protonmail.com**. Do **not** open a public issue for
security reports.

You will receive an acknowledgement within **72 hours**. Coordinated
disclosure: we ask for up to 90 days before public disclosure while a
patch ships.

## Scope notes

`sheet-core` is an **in-memory data model**: it holds cells and a
dependency graph over formula text. Security considerations for
integrators:

- **No I/O surface.** The crate never touches the filesystem, network, or
  processes. Untrusted data can only enter through `&str`/`&[u8]`-shaped
  API parameters (`extract_refs`, `add_formula`, …).
- **Totality on hostile input.** Reference extraction is designed to
  reject or skip arbitrary bytes with a typed error: fuzzing asserts
  `Err`-or-`Ok`, never a panic. Formula text is never evaluated: the
  crate parses *references*, not expressions, so formula-injection is
  out of scope by construction.
- **Denial-of-service bounds.** Range references are capped at
  `RANGE_CELL_CAP` (65,536) expanded cells; larger rectangles are
  rejected rather than materialized. Graph algorithms are iterative
  (no recursion-depth blowup on 500-cell adversarial chains).
- `#![forbid(unsafe_code)]`: no unsafe blocks exist in this crate.

[GitHub security advisories]:
    https://github.com/WyattAu/sheet-core/security/advisories/new
