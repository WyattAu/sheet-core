//! Golden-style integration tests — deep chains, diamonds, mixed
//! constants, and the documented API behaviors that back the README
//! claims. Complements the property tests with exact, hand-computed
//! expectations.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(clippy::indexing_slicing)]

use sheet_core::refs::{cell_name, extract_refs, parse_cell_name, RANGE_CELL_CAP};
use sheet_core::{Cell, CellValue, DependencyGraph, ExcelError, GraphError, Sheet};
use std::vec::Vec;

/// 500-cell chain: the iterative Tarjan and Kahn implementations must not
/// overflow any stack, and the order must stay exact.
#[test]
fn deep_chain_orders_without_recursion_limits() {
    const N: u32 = 500;
    let mut g = DependencyGraph::new();
    for i in (0..N).rev() {
        // Each cell reads its successor; add in reverse so insertion order
        // differs from dependency order.
        let formula = if i + 1 < N {
            format!("={}", cell_name((i + 1, 0)))
        } else {
            String::from("=1")
        };
        g.add_formula((i, 0), &formula).unwrap();
    }
    let order = g.topological_order();
    assert_eq!(order.len(), N as usize);
    for (k, &cell) in order.iter().enumerate() {
        // Cell i reads cell i+1, so the deepest precedent evaluates first.
        assert_eq!(
            cell,
            (N - 1 - k as u32, 0),
            "position {k} wrong in {order:?}"
        );
    }
    assert!(g.has_circular().is_empty());

    // Editing the last cell dirties the whole chain in order.
    let dirty = g.dirty_subgraph(&[(N - 1, 0)]);
    assert_eq!(dirty, g.topological_order());
}

/// A long self-referential ring: every member is one cycle.
#[test]
fn ring_reports_one_cycle_of_every_member() {
    const N: u32 = 100;
    let mut g = DependencyGraph::new();
    for i in 0..N {
        let next = (i + 1) % N;
        g.add_formula((i, 0), &format!("={}", cell_name((next, 0))))
            .unwrap();
    }
    let cycles = g.has_circular();
    assert_eq!(cycles.len(), 1);
    let cycle = &cycles[0];
    assert_eq!(cycle.len(), N as usize);
    // Sorted ascending: first member is (0,0), last is (99,0).
    assert_eq!(cycle[0], (0, 0));
    assert_eq!(cycle[(N as usize) - 1], (N - 1, 0));
    assert!(g.topological_order().is_empty());
}

/// The README workflow end to end: sheet + graph together.
#[test]
fn sheet_and_graph_work_together() {
    let mut sheet = Sheet::new();
    sheet.set(0, 0, Cell::number(41.0)); // A1
    sheet.set(0, 1, Cell::number(1.0)); // B1
    sheet.set(1, 0, Cell::formula("=A1+B1")); // A2
    sheet.set(2, 0, Cell::formula("=A2*2")); // A3
    sheet.set(3, 0, Cell::error(ExcelError::Ref)); // A4

    assert_eq!(sheet.used_range(), Some(((0, 0), (3, 1))));

    let mut g = DependencyGraph::new();
    for (coord, cell) in sheet.iter() {
        if let CellValue::Formula(text) = &cell.value {
            g.add_formula(*coord, text).unwrap();
        }
    }
    assert_eq!(g.formula_count(), 2);
    assert_eq!(g.topological_order(), vec![(1, 0), (2, 0)]);
    // Touching A1 (or B1) recomputes A2 then A3.
    assert_eq!(g.dirty_subgraph(&[(0, 0)]), vec![(1, 0), (2, 0)]);
    assert_eq!(g.dirty_subgraph(&[(0, 1)]), vec![(1, 0), (2, 0)]);
    // Overwriting A2's formula with a constant breaks the chain.
    sheet.set(1, 0, Cell::number(0.0));
    g.remove_formula((1, 0));
    assert_eq!(g.dirty_subgraph(&[(0, 0)]), Vec::<(u32, u32)>::new());
    assert_eq!(g.dirty_subgraph(&[(1, 0)]), vec![(2, 0)]);
}

#[test]
fn error_values_display_and_compare() {
    let mut sheet = Sheet::new();
    sheet.set(0, 0, Cell::new(CellValue::Error(ExcelError::Div0)));
    assert_eq!(
        sheet.get(0, 0).map(|c| match c.value {
            CellValue::Error(e) => String::from(e.as_str()),
            _ => String::from("wrong"),
        }),
        Some(String::from("#DIV/0!"))
    );
    assert_eq!(
        CellValue::Error(ExcelError::Na),
        CellValue::Error(ExcelError::Na)
    );
}

#[test]
fn extraction_is_lenient_on_junk_but_strict_on_qualifiers() {
    // `1A` is jammed junk: skipped silently (lenient contract), while the
    // qualified ref still extracts.
    let refs = extract_refs("= 1A + Sheet2!B2").unwrap();
    assert_eq!(refs, vec![(1, 1)]);

    // A dangling qualifier is an error at the byte after `!` in the body.
    let err = extract_refs("=SUM(A1)+Sheet9!").unwrap_err();
    assert!(
        matches!(err, GraphError::InvalidReference { position: 15, .. }),
        "{err:?}"
    );
}

#[test]
fn cell_name_and_parse_are_inverse() {
    for coord in [(0u32, 0u32), (12, 34), (1_048_575, 16_383), (7, 6)] {
        let name = cell_name(coord);
        assert_eq!(parse_cell_name(&name), Ok(coord), "{name} not inverse");
    }
    // Absolute spellings parse but are not produced by cell_name.
    assert_eq!(parse_cell_name("$AB$12"), Ok((11, 27)));
}

#[test]
fn cap_boundary_is_exact() {
    // 65_536 cells exactly: accepted.
    let refs = extract_refs("=A1:IV256").unwrap();
    assert_eq!(refs.len() as u64, RANGE_CELL_CAP);
    // One more row: rejected.
    let err = extract_refs("=A1:IV257").unwrap_err();
    assert!(
        matches!(err, GraphError::RangeTooLarge { cells: 65_792, .. }),
        "{err:?}"
    );
}

#[test]
fn sheet_bounds_survive_pathological_removals() {
    let mut sheet = Sheet::new();
    for i in 0..10u32 {
        sheet.set(i, i, Cell::number(f64::from(i)));
    }
    assert_eq!(sheet.used_range(), Some(((0, 0), (9, 9))));
    // Remove the whole leading edge, then the trailing edge.
    for i in 0..5u32 {
        sheet.remove(i, i);
    }
    assert_eq!(sheet.used_range(), Some(((5, 5), (9, 9))));
    for i in 5..10u32 {
        sheet.remove(i, i);
    }
    assert_eq!(sheet.used_range(), None);
    assert_eq!(sheet.len(), 0);
    // Reinsert after full removal keeps bounds correct.
    sheet.set(2, 3, Cell::number(1.0));
    assert_eq!(sheet.used_range(), Some(((2, 3), (2, 3))));
}
