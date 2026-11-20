# Pattern adoption

Declared per the pattern canon adoption format; every claim is verified against code in this repo.

| Pattern | Status | Evidence |
|---|---|---|
| T13 | adopted | src/lib.rs:6 (no file format knowledge; the codec is a downstream crate) |
| T1 | partial | src/lib.rs:38 (lenient single-pass extraction; malformed input is a typed error) |
| P14 | partial | src/graph.rs:13 (deterministic evaluation order: same edges, same order) |
