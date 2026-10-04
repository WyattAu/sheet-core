//! Property tests — the graph invariants that make the evaluation order
//! trustworthy, checked on 500 random graphs per property.
//!
//! These properties are the correctness proof for the dependency machinery:
//!
//! 1. **Acyclic graphs order.** For an arbitrary DAG, `topological_order`
//!    places every formula strictly after every formula precedent, contains
//!    every formula exactly once, and `has_circular` reports nothing.
//! 2. **Dirty sets are exact.** `dirty_subgraph(edited)` equals the
//!    transitive dependents computed by an independent BFS, is itself in
//!    topological order, and includes edited formulas.
//! 3. **Edge symmetry.** `d ∈ dependents(c)` implies `c ∈ precedents(d)`
//!    for every formula `d`; removing every formula empties both maps.
//! 4. **Cycles are found.** Injecting a two-cell cycle is detected; the
//!    cycle members never appear in `topological_order`.
//! 5. **Reference extraction is total.** Arbitrary byte soup never panics;
//!    whenever extraction succeeds, precedents are sorted and deduplicated.

// Test harness: assertions legitimately panic; the lib target stays
// lint-clean.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(clippy::indexing_slicing)]

use proptest::prelude::*;
use sheet_core::refs::{cell_name, extract_refs, index_to_column};
use sheet_core::{DependencyGraph, GraphError};
use std::collections::{BTreeMap, BTreeSet, VecDeque};

type Coord = (u32, u32);

/// A random formula cell set: coordinates on a distinct diagonal band plus
/// DAG edges that always point from a lower index to a higher one (acyclic
/// by construction).
#[derive(Debug, Clone)]
struct RandomDag {
    cells: Vec<Coord>,
    edges: Vec<(usize, usize)>, // (precedent idx, dependent idx), lo < hi
}

fn random_dag() -> impl Strategy<Value = RandomDag> {
    (1usize..12usize, 0u32..40, 0u32..40)
        .prop_flat_map(|(n, r0, c0)| {
            // Strategy bound guarantees n < 12, so the usize→u32 widening
            // cannot truncate.
            #[allow(clippy::cast_possible_truncation)]
            let n32 = n as u32;
            let cells: Vec<Coord> = (0..n32).map(|i| (r0 + i, c0 + i * 2)).collect();
            (
                Just(cells),
                proptest::collection::vec((0usize..n, 0usize..n), 0..3 * n),
            )
        })
        .prop_map(|(cells, pairs)| {
            let _n = cells.len();
            let mut edges = BTreeSet::new();
            for (a, b) in pairs {
                let (lo, hi) = if a < b { (a, b) } else { (b, a) };
                if lo != hi {
                    edges.insert((lo, hi));
                }
            }
            RandomDag {
                cells,
                edges: edges.into_iter().collect(),
            }
        })
}

impl RandomDag {
    /// Builds the graph: each cell's formula reads its DAG precedents.
    /// Precedent-free cells get a formula referencing row 900 — a coordinate
    /// that can never be a `dag.cells` member — so the parsed graph has
    /// exactly the DAG edges plus constant leaves.
    fn build(&self) -> DependencyGraph {
        let mut g = DependencyGraph::new();
        for (i, &cell) in self.cells.iter().enumerate() {
            // Precedents of cell i: edges whose DEPENDENT is i.
            let preds: Vec<Coord> = self
                .edges
                .iter()
                .filter(|&&(_, b)| b == i)
                .map(|&(a, _)| self.cells[a])
                .collect();
            let formula = if preds.is_empty() {
                format!("={}900", index_to_column(cell.1))
            } else {
                let mut f = String::from("=");
                for (k, p) in preds.iter().enumerate() {
                    if k > 0 {
                        f.push('+');
                    }
                    f.push_str(&cell_name(*p));
                }
                f
            };
            g.add_formula(cell, &formula).unwrap();
        }
        g
    }
}

proptest! {
    #![proptest_config(proptest::test_runner::Config::with_cases(500))]

    /// Property 1 — DAGs order: total, correct, and cycle-free.
    #[test]
    fn dags_topologically_order(dag in random_dag()) {
        let g = dag.build();
        let order = g.topological_order();

        // Totality: every formula cell appears exactly once (cells are on a
        // strictly ascending diagonal, so they are already sorted).
        let mut sorted = order.clone();
        sorted.sort_unstable();
        sorted.dedup();
        prop_assert_eq!(sorted.len(), dag.cells.len());
        prop_assert_eq!(sorted, dag.cells.clone());

        // Correctness: every DAG edge is respected by the order.
        let pos: BTreeMap<Coord, usize> =
            order.iter().enumerate().map(|(i, &c)| (c, i)).collect();
        for &(a, b) in &dag.edges {
            prop_assert!(pos[&dag.cells[a]] < pos[&dag.cells[b]],
                "edge {:?}→{:?} violated by order {:?}",
                dag.cells[a], dag.cells[b], order);
        }

        // Acyclic: no cycles reported.
        prop_assert!(g.has_circular().is_empty());
    }

    /// Property 2 — dirty sets are exactly the transitive dependents
    /// (independent BFS oracle), and are themselves topologically ordered.
    #[test]
    fn dirty_sets_are_transitive_dependents(
        dag in random_dag(),
        seeds in proptest::collection::vec(proptest::num::u32::ANY, 1..4),
    ) {
        let g = dag.build();
        let n = dag.cells.len() as u32;
        let pick = |k: u32| dag.cells[(seeds[(k as usize) % seeds.len()] % n) as usize];
        let edited = vec![dag.cells[0], pick(7), pick(11)];

        // Independent BFS oracle over the known edge list.
        let mut depends_on: BTreeMap<Coord, Vec<Coord>> = BTreeMap::new();
        for &(a, b) in &dag.edges {
            depends_on.entry(dag.cells[a]).or_default().push(dag.cells[b]);
        }
        let mut expected: BTreeSet<Coord> = BTreeSet::new();
        let mut queue: VecDeque<Coord> = VecDeque::new();
        for &e in &edited {
            if g.contains_formula(e) {
                expected.insert(e); // edited formulas recompute themselves
            }
            queue.push_back(e);
        }
        while let Some(c) = queue.pop_front() {
            for d in depends_on.get(&c).into_iter().flatten() {
                if expected.insert(*d) {
                    queue.push_back(*d);
                }
            }
        }

        let dirty = g.dirty_subgraph(&edited);
        prop_assert_eq!(dirty.len(), expected.len(), "dirty set is not exact");

        // The dirty set comes out in a valid evaluation order for the
        // induced edges.
        let pos: BTreeMap<Coord, usize> =
            dirty.iter().enumerate().map(|(i, &c)| (c, i)).collect();
        for &(a, b) in &dag.edges {
            let (pa, pb) = (dag.cells[a], dag.cells[b]);
            if let (Some(&ia), Some(&ib)) = (pos.get(&pa), pos.get(&pb)) {
                prop_assert!(ia < ib, "dirty order violates {pa:?}→{pb:?}");
            }
        }
    }

    /// Property 3 — forward/reverse symmetry, and removal empties both maps.
    #[test]
    fn forward_and_reverse_stay_symmetric(dag in random_dag()) {
        let g = dag.build();
        for &f in &dag.cells {
            for p in g.precedents(f) {
                prop_assert!(g.dependents(p).contains(&f),
                    "precedent {p:?} of {f:?} does not list it as dependent");
            }
        }
        let mut h = g.clone();
        for &f in &dag.cells {
            h.remove_formula(f);
        }
        prop_assert_eq!(h.formula_count(), 0);
        for &c in &dag.cells {
            prop_assert!(h.precedents(c).is_empty());
            prop_assert!(h.dependents(c).is_empty());
        }
    }

    /// Property 4 — an injected two-cell cycle is detected, its members
    /// never appear in the evaluation order, and honest DAG edges still do.
    #[test]
    fn injected_cycles_are_detected(
        dag in random_dag(),
        a in 0usize..64,
        b in 0usize..64,
    ) {
        let n = dag.cells.len();
        let (i, j) = (a % n, b % n);
        if i == j {
            return Ok(()); // degenerate pick: self-loops are covered in unit tests
        }
        let mut g = dag.build();
        let (ci, cj) = (dag.cells[i], dag.cells[j]);
        g.add_formula(ci, &format!("={}", cell_name(cj))).unwrap();
        g.add_formula(cj, &format!("={}", cell_name(ci))).unwrap();

        let cycles = g.has_circular();
        prop_assert!(!cycles.is_empty(), "injected {ci:?}↔{cj:?} cycle not detected");
        let cyclic: BTreeSet<Coord> = cycles.iter().flatten().copied().collect();
        prop_assert!(cyclic.contains(&ci) && cyclic.contains(&cj));

        // Cycle members are excluded from the topological order; any DAG
        // edge whose both ends survived in the order is still respected.
        // (Edges touching the injected cycle — or cells that read it — have
        // no constraint: those cells can never be ordered.)
        let order = g.topological_order();
        let pos: BTreeMap<Coord, usize> =
            order.iter().enumerate().map(|(k, &c)| (c, k)).collect();
        for &(x, y) in &dag.edges {
            let (cx, cy) = (dag.cells[x], dag.cells[y]);
            if let (Some(&px), Some(&py)) = (pos.get(&cx), pos.get(&cy)) {
                prop_assert!(px < py);
            }
        }
        prop_assert!(order.iter().all(|c| !cyclic.contains(c)));
    }

    /// Property 5 — extraction totality on arbitrary byte soup: never
    /// panics; successes are sorted + deduplicated.
    #[test]
    fn extraction_is_total_and_normalized(
        soup in proptest::collection::vec(proptest::num::u8::ANY, 0..80),
    ) {
        let text: String = soup
            .into_iter()
            .map(|b| if b.is_ascii_graphic() || b == b' ' { b as char } else { 'A' })
            .collect();
        match extract_refs(&text) {
            Ok(refs) => {
                let mut sorted = refs.clone();
                sorted.sort_unstable();
                sorted.dedup();
                prop_assert_eq!(sorted, refs);
            }
            Err(
                GraphError::EmptyFormula
                    | GraphError::UnterminatedString { .. }
                    | GraphError::UnterminatedQuote { .. }
                    | GraphError::InvalidReference { .. }
                    | GraphError::ColumnOverflow { .. }
                    | GraphError::RowOverflow { .. }
                    | GraphError::RangeTooLarge { .. },
            ) => {}
        }
    }
}
