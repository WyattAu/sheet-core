//! The dependency DAG — formulas, precedents, dependents, evaluation order.
//!
//! The graph is two index maps over one source of truth:
//!
//! - **forward**: formula cell → the cells its formula reads (precedents),
//! - **reverse**: cell → the formula cells that read it (dependents).
//!
//! Both directions are maintained on every [`add`](DependencyGraph::add_formula)
//! and [`remove`](DependencyGraph::remove_formula), so the two queries are
//! always consistent by construction — a property the test suite checks on
//! random graphs.
//!
//! # Determinism
//!
//! Every derived quantity — [`topological_order`](DependencyGraph::topological_order),
//! [`dirty_subgraph`](DependencyGraph::dirty_subgraph),
//! [`has_circular`](DependencyGraph::has_circular) — is a pure function of
//! the edge set: Kahn's algorithm with a sorted ready set and Tarjan SCC
//! with ascending neighbor order. Same edges in, same order out, on every
//! machine, every run.
//!
//! # Cycle policy
//!
//! Cycles are *represented*, not rejected at insert: real workbooks contain
//! them, and an engine needs [`has_circular`] to report them while the rest
//! of the graph still evaluates. Cycle members are excluded from
//! [`topological_order`] (no valid position exists for them) and from
//! [`dirty_subgraph`] roots' propagation only insofar as they cannot be
//! ordered — they still appear when the edit reaches them.

use crate::error::GraphError;
use crate::refs::extract_refs;
use alloc::collections::{BTreeMap, BTreeSet};
use alloc::vec::Vec;

/// The dependency graph of one worksheet's formulas.
///
/// A cell is a *formula cell* iff it appears in the forward map; constant
/// cells appear only on the reverse side (as things formulas read). The
/// graph is single-sheet: coordinates are `(row, col)` within the modeled
/// worksheet (see [`crate::refs`] for the sheet-qualifier policy).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DependencyGraph {
    /// formula cell → precedents (cells its formula reads).
    forward: BTreeMap<(u32, u32), BTreeSet<(u32, u32)>>,
    /// cell → dependents (formula cells whose formulas read it).
    reverse: BTreeMap<(u32, u32), BTreeSet<(u32, u32)>>,
}

impl DependencyGraph {
    /// An empty graph.
    #[must_use]
    pub const fn new() -> Self {
        DependencyGraph {
            forward: BTreeMap::new(),
            reverse: BTreeMap::new(),
        }
    }

    /// Extracts the references of `formula` into a standalone graph anchored
    /// at the origin cell `(0, 0)` — i.e. exactly
    /// `Self::new()` + [`add_formula`](Self::add_formula)`((0, 0), formula)`.
    ///
    /// Useful for one-shot parsing (`precedents((0, 0))` returns the sorted,
    /// de-duplicated references) and as the typed entry point when the
    /// anchor cell is irrelevant. Note the modeling consequence: a formula
    /// at the origin that reads `A1` produces a self-edge and will be
    /// reported by [`has_circular`](Self::has_circular) — correct for the
    /// anchor, so prefer [`add_formula`](Self::add_formula) with the real
    /// address when one exists.
    ///
    /// # Errors
    ///
    /// [`GraphError`] — see [`extract_refs`](crate::refs::extract_refs).
    pub fn extract(formula: &str) -> Result<Self, GraphError> {
        let mut graph = Self::new();
        graph.add_formula((0, 0), formula)?;
        Ok(graph)
    }

    /// Registers `cell` as holding `formula`, replacing any formula
    /// previously registered there. The graph is mutated only when the
    /// formula parses — a rejected formula leaves it untouched.
    ///
    /// # Errors
    ///
    /// [`GraphError`] — see [`extract_refs`](crate::refs::extract_refs).
    pub fn add_formula(&mut self, cell: (u32, u32), formula: &str) -> Result<(), GraphError> {
        let refs = extract_refs(formula)?;
        self.remove_formula(cell); // replace semantics: detach old edges first
        let set: BTreeSet<(u32, u32)> = refs.into_iter().collect();
        for precedent in &set {
            self.reverse.entry(*precedent).or_default().insert(cell);
        }
        self.forward.insert(cell, set);
        Ok(())
    }

    /// Unregisters the formula at `cell` (no-op when there is none). The
    /// cell's *own* outgoing edges go; incoming edges of other formulas
    /// that read it stay — removing a formula does not blank its value for
    /// readers, it just stops the cell from being recomputed.
    pub fn remove_formula(&mut self, cell: (u32, u32)) {
        if let Some(precedents) = self.forward.remove(&cell) {
            for precedent in precedents {
                if let Some(dependents) = self.reverse.get_mut(&precedent) {
                    dependents.remove(&cell);
                    if dependents.is_empty() {
                        self.reverse.remove(&precedent);
                    }
                }
            }
        }
    }

    /// Cells read by the formula at `cell`, sorted ascending. Empty when
    /// `cell` holds no formula.
    #[must_use]
    pub fn precedents(&self, cell: (u32, u32)) -> Vec<(u32, u32)> {
        self.forward
            .get(&cell)
            .map_or_else(Vec::new, |p| p.iter().copied().collect())
    }

    /// Formula cells whose formulas read `cell`, sorted ascending.
    #[must_use]
    pub fn dependents(&self, cell: (u32, u32)) -> Vec<(u32, u32)> {
        self.reverse
            .get(&cell)
            .map_or_else(Vec::new, |d| d.iter().copied().collect())
    }

    /// `true` when `cell` holds a registered formula.
    #[must_use]
    pub fn contains_formula(&self, cell: (u32, u32)) -> bool {
        self.forward.contains_key(&cell)
    }

    /// Number of registered formulas.
    #[must_use]
    pub fn formula_count(&self) -> usize {
        self.forward.len()
    }

    /// All registered formula cells, sorted ascending.
    #[must_use]
    pub fn formula_cells(&self) -> Vec<(u32, u32)> {
        self.forward.keys().copied().collect()
    }

    /// The evaluation order: every formula cell, ordered so that each
    /// formula comes after every formula it (transitively) reads. Ties
    /// break by ascending `(row, col)`, so the result is a total, stable
    /// order. Formula cells inside a circular reference have no valid
    /// position — and neither does anything that reads them — so both are
    /// omitted; pair with [`has_circular`](Self::has_circular) to detect
    /// that case.
    #[must_use]
    pub fn topological_order(&self) -> Vec<(u32, u32)> {
        // in-degree over formula cells only: an edge p → f exists for every
        // precedent p of f that is itself a formula cell.
        let mut in_degree: BTreeMap<(u32, u32), usize> = BTreeMap::new();
        let mut edges: BTreeMap<(u32, u32), Vec<(u32, u32)>> = BTreeMap::new();
        for (&formula, precedents) in &self.forward {
            in_degree.entry(formula).or_insert(0);
            for &p in precedents {
                if self.forward.contains_key(&p) {
                    *in_degree.entry(formula).or_insert(0) += 1;
                    edges.entry(p).or_default().push(formula);
                }
            }
        }

        // Kahn's algorithm; the ready set is sorted, so pops are deterministic.
        let mut ready: BTreeSet<(u32, u32)> = in_degree
            .iter()
            .filter(|(_, &d)| d == 0)
            .map(|(&f, _)| f)
            .collect();
        let mut order = Vec::with_capacity(self.forward.len());
        while let Some(cell) = ready.pop_first() {
            order.push(cell);
            if let Some(targets) = edges.get(&cell) {
                for &t in targets {
                    if let Some(d) = in_degree.get_mut(&t) {
                        *d -= 1;
                        if *d == 0 {
                            ready.insert(t);
                        }
                    }
                }
            }
        }
        order
    }

    /// The minimal recompute set for an edit of `edited`, in evaluation
    /// order — ready to drive an incremental recalc directly.
    ///
    /// Formulas among the edited cells are included (an edited formula must
    /// re-evaluate); constants only propagate. Cycle members that the
    /// propagation reaches are included but cannot be ordered, so they are
    /// appended after the orderable prefix (see [`has_circular`](Self::has_circular)).
    #[must_use]
    pub fn dirty_subgraph(&self, edited: &[(u32, u32)]) -> Vec<(u32, u32)> {
        // Transitive dependents of the edited cells (reverse BFS), keeping
        // only formula cells; edited formula cells are dirty themselves.
        let mut dirty: BTreeSet<(u32, u32)> = BTreeSet::new();
        let mut frontier: Vec<(u32, u32)> = Vec::new();
        for &cell in edited {
            if self.forward.contains_key(&cell) {
                if dirty.insert(cell) {
                    frontier.push(cell);
                }
            } else {
                frontier.push(cell); // constant: propagates without being dirty
            }
        }
        while let Some(cell) = frontier.pop() {
            if let Some(dependents) = self.reverse.get(&cell) {
                for &d in dependents {
                    if dirty.insert(d) {
                        frontier.push(d);
                    }
                }
            }
        }

        // Order the dirty set internally, appending anything circular.
        let orderable = self.topological_order();
        let mut result: Vec<(u32, u32)> = orderable
            .into_iter()
            .filter(|c| dirty.contains(c))
            .collect();
        for &c in &dirty {
            if !result.contains(&c) {
                result.push(c);
            }
        }
        result
    }

    /// The circular references, as explicit cycles. Each cycle is sorted
    /// ascending and the cycles themselves are sorted by first member, so
    /// the output is deterministic. Self-references (`A1 = A1+1`) report as
    /// single-member cycles. Empty when the graph is acyclic.
    #[must_use]
    pub fn has_circular(&self) -> Vec<Vec<(u32, u32)>> {
        let sccs = tarjan_sccs(self);
        let mut cycles = Vec::new();
        for scc in sccs {
            let self_loop = scc
                .first()
                .is_some_and(|&m| self.forward.get(&m).is_some_and(|p| p.contains(&m)));
            if scc.len() > 1 || self_loop {
                cycles.push(scc);
            }
        }
        cycles.sort();
        cycles
    }
}

/// Tarjan strongly-connected components over the formula-cell subgraph
/// (edges: formula → formula precedents). Iterative (no recursion, so no
/// stack overflow on deep chains); neighbor iteration is ascending, and
/// components are emitted sorted, making the result deterministic.
fn tarjan_sccs(graph: &DependencyGraph) -> Vec<Vec<(u32, u32)>> {
    #[derive(Clone, Copy, PartialEq)]
    enum Frame {
        Enter,
        Visit { child: usize },
    }

    let mut index: BTreeMap<(u32, u32), usize> = BTreeMap::new();
    let mut low: BTreeMap<(u32, u32), usize> = BTreeMap::new();
    let mut on_stack: BTreeSet<(u32, u32)> = BTreeSet::new();
    let mut stack: Vec<(u32, u32)> = Vec::new();
    let mut next_index = 0usize;
    let mut components: Vec<Vec<(u32, u32)>> = Vec::new();

    for &root in graph.forward.keys() {
        if index.contains_key(&root) {
            continue;
        }
        let mut call = alloc::vec![(root, Frame::Enter)];
        while let Some((cell, frame)) = call.pop() {
            match frame {
                Frame::Enter => {
                    index.insert(cell, next_index);
                    low.insert(cell, next_index);
                    next_index += 1;
                    stack.push(cell);
                    on_stack.insert(cell);
                    call.push((cell, Frame::Visit { child: 0 }));
                }
                Frame::Visit { child } => {
                    let precedents = graph
                        .forward
                        .get(&cell)
                        .map_or_else(Vec::new, |p| p.iter().copied().collect::<Vec<_>>());
                    if let Some(&next) = precedents.get(child) {
                        call.push((cell, Frame::Visit { child: child + 1 }));
                        if !graph.forward.contains_key(&next) {
                            continue;
                        }
                        if index.contains_key(&next) {
                            if on_stack.contains(&next) {
                                if let (Some(&n), Some(&l)) = (index.get(&next), low.get(&cell)) {
                                    let m = n.min(l);
                                    low.insert(cell, m);
                                }
                            }
                        } else {
                            call.push((next, Frame::Enter));
                        }
                    } else {
                        // All children processed: maybe close a component.
                        if let (Some(&l), Some(&i)) = (low.get(&cell), index.get(&cell)) {
                            if l == i {
                                let mut component = Vec::new();
                                while let Some(top) = stack.pop() {
                                    on_stack.remove(&top);
                                    component.push(top);
                                    if top == cell {
                                        break;
                                    }
                                }
                                component.sort_unstable();
                                components.push(component);
                            }
                        }
                        // Propagate low-link to the parent.
                        if let Some(&l) = low.get(&cell) {
                            if let Some(&(p, Frame::Visit { .. })) = call.last() {
                                if let Some(&pl) = low.get(&p) {
                                    low.insert(p, pl.min(l));
                                }
                            }
                        }
                    }
                }
            }
        }
    }
    components
}

#[cfg(test)]
mod tests {
    #![allow(
        clippy::unwrap_used,
        clippy::expect_used,
        clippy::panic,
        clippy::indexing_slicing
    )]

    use super::DependencyGraph;
    use crate::error::GraphError;
    use alloc::{string::String, vec, vec::Vec};

    #[test]
    fn add_query_remove_round_trip() {
        let mut g = DependencyGraph::new();
        assert_eq!(g.formula_count(), 0);
        assert!(g.formula_cells().is_empty());
        g.add_formula((0, 1), "=A1+B2").unwrap();
        assert!(g.contains_formula((0, 1)));
        assert_eq!(g.formula_count(), 1);
        assert_eq!(g.precedents((0, 1)), vec![(0, 0), (1, 1)]);
        assert_eq!(g.dependents((0, 0)), vec![(0, 1)]);
        assert_eq!(g.dependents((1, 1)), vec![(0, 1)]);
        assert_eq!(g.precedents((0, 0)), Vec::<(u32, u32)>::new()); // constant
        assert_eq!(g.formula_cells(), vec![(0, 1)]);

        g.remove_formula((0, 1));
        assert!(!g.contains_formula((0, 1)));
        assert_eq!(g.formula_count(), 0);
        assert_eq!(g.dependents((0, 0)), Vec::<(u32, u32)>::new());
        g.remove_formula((9, 9)); // absent: no-op
    }

    #[test]
    fn replace_semantics_swap_edges() {
        let mut g = DependencyGraph::new();
        g.add_formula((0, 0), "=B1").unwrap();
        let _ = g.add_formula((0, 0), "=C1"); // replaces B1 edge with C1
        assert_eq!(g.precedents((0, 0)), vec![(0, 2)]);
        assert_eq!(g.dependents((0, 1)), Vec::<(u32, u32)>::new());
        assert_eq!(g.dependents((0, 2)), vec![(0, 0)]);

        // Failed parse leaves the graph untouched.
        assert_eq!(
            g.add_formula((0, 0), "=Nope!"),
            Err(GraphError::InvalidReference {
                position: 5,
                text: String::new()
            })
        );
        assert_eq!(g.precedents((0, 0)), vec![(0, 2)]);
    }

    #[test]
    fn topological_order_places_formulas_after_their_inputs() {
        let mut g = DependencyGraph::new();
        // B2 = A1+1, C3 = B2+1, D4 = C3+1 (a pure chain), E5 = const ref.
        g.add_formula((1, 1), "=A1+1").unwrap();
        g.add_formula((2, 2), "=B2+1").unwrap();
        g.add_formula((3, 3), "=C4+1").unwrap();
        let order = g.topological_order();
        assert_eq!(order, vec![(1, 1), (2, 2), (3, 3)]);

        // Diamond: A1 → B2, A1 → C3, B2 + C3 → D4.
        let mut d = DependencyGraph::new();
        d.add_formula((1, 1), "=A1").unwrap();
        d.add_formula((2, 2), "=A1").unwrap();
        d.add_formula((3, 3), "=B2+C3").unwrap();
        let order = d.topological_order();
        assert_eq!(order.len(), 3);
        let pos = |c| order.iter().position(|&x| x == c).unwrap();
        assert!(pos((1, 1)) < pos((3, 3)));
        assert!(pos((2, 2)) < pos((3, 3)));
    }

    #[test]
    fn dirty_subgraph_is_minimal_transitive_and_ordered() {
        let mut g = DependencyGraph::new();
        g.add_formula((1, 1), "=A1").unwrap(); // B2
        g.add_formula((2, 2), "=B2").unwrap(); // C3
        g.add_formula((3, 3), "=Z9").unwrap(); // D4 (unrelated)

        // Editing the constant A1 dirties B2 then C3, in order.
        assert_eq!(g.dirty_subgraph(&[(0, 0)]), vec![(1, 1), (2, 2)]);
        // Editing B2 (a formula) dirties itself and downstream.
        assert_eq!(g.dirty_subgraph(&[(1, 1)]), vec![(1, 1), (2, 2)]);
        // Unrelated edits dirty nothing.
        assert_eq!(g.dirty_subgraph(&[(8, 8)]), Vec::<(u32, u32)>::new());
        // Empty edit list.
        assert_eq!(g.dirty_subgraph(&[]), Vec::<(u32, u32)>::new());
    }

    #[test]
    fn circular_references_are_reported_as_cycles() {
        let mut g = DependencyGraph::new();
        g.add_formula((0, 0), "=B1").unwrap();
        g.add_formula((0, 1), "=A1").unwrap(); // A1 ↔ B1
        g.add_formula((5, 5), "=A1").unwrap(); // feeds into the cycle but not in it
        let cycles = g.has_circular();
        assert_eq!(cycles, vec![vec![(0, 0), (0, 1)]]);

        // Self-reference is a single-member cycle.
        let mut s = DependencyGraph::new();
        s.add_formula((2, 2), "=C3+1").unwrap();
        assert_eq!(s.has_circular(), vec![vec![(2, 2)]]);

        // Cycle members are omitted from the evaluation order — and so is
        // everything downstream of a cycle (it could never evaluate first).
        let order = g.topological_order();
        assert_eq!(order, Vec::<(u32, u32)>::new());
    }

    #[test]
    fn extract_anchors_at_the_origin() {
        let g = DependencyGraph::extract("=SUM(B1:C2)").unwrap();
        assert_eq!(g.precedents((0, 0)), vec![(0, 1), (0, 2), (1, 1), (1, 2)]);
        assert!(DependencyGraph::extract("=Nope!1").is_err());
    }
}
