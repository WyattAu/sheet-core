//! Sparse worksheet storage — `BTreeMap<(row, col), Cell>` plus tracked
//! bounds.
//!
//! Absence *is* the blank: a coordinate not in the map holds nothing, so
//! memory scales with filled cells, not with the bounding rectangle. The
//! map also gives deterministic iteration order (row-major) for free —
//! golden tests and serialized output come out in a stable order.

use crate::cell::Cell;
use alloc::vec::Vec;
use core::ops::RangeInclusive;

/// One worksheet's cells: sparse, row-major-ordered, unbounded up to the
/// `u32 × u32` coordinate space.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Sheet {
    cells: alloc::collections::BTreeMap<(u32, u32), Cell>,
    max_row: u32,
    max_col: u32,
}

impl Sheet {
    /// An empty sheet.
    #[must_use]
    pub const fn new() -> Self {
        Sheet {
            cells: alloc::collections::BTreeMap::new(),
            max_row: 0,
            max_col: 0,
        }
    }

    /// Sets (inserts or overwrites) the cell at `row`, `col`.
    pub fn set(&mut self, row: u32, col: u32, cell: Cell) {
        self.cells.insert((row, col), cell);
        self.max_row = self.max_row.max(row);
        self.max_col = self.max_col.max(col);
    }

    /// Returns the cell at `row`, `col`, or `None` when the coordinate is
    /// blank (absent).
    #[must_use]
    pub fn get(&self, row: u32, col: u32) -> Option<&Cell> {
        self.cells.get(&(row, col))
    }

    /// Removes the cell at `row`, `col` (making it blank). Removing an
    /// absent coordinate is a no-op.
    pub fn remove(&mut self, row: u32, col: u32) {
        if self.cells.remove(&(row, col)).is_some() && (row == self.max_row || col == self.max_col)
        {
            self.recompute_bounds();
        }
    }

    /// The used range — `((min_row, min_col), (max_row, max_col))` over all
    /// present cells — or `None` when the sheet is empty. The spreadsheet
    /// equivalent of the `dimension` element: the bounding box of occupied
    /// coordinates.
    #[must_use]
    pub fn used_range(&self) -> Option<((u32, u32), (u32, u32))> {
        if self.cells.is_empty() {
            return None;
        }
        let mut min = (u32::MAX, u32::MAX);
        for &(r, c) in self.cells.keys() {
            min = (min.0.min(r), min.1.min(c));
        }
        Some((min, (self.max_row, self.max_col)))
    }

    /// Iterates present cells in row-major (deterministic) order.
    pub fn iter(&self) -> alloc::collections::btree_map::Iter<'_, (u32, u32), Cell> {
        self.cells.iter()
    }
    /// Number of present (non-blank-by-absence) cells.
    #[must_use]
    pub fn len(&self) -> usize {
        self.cells.len()
    }

    /// `true` when no cell is present.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.cells.is_empty()
    }

    /// Collects the present cells in the given (row, col) range, in
    /// row-major order. Range-bounded scans back the file formats' row
    /// streaming without materializing blank coordinates.
    #[must_use]
    pub fn cells_in(
        &self,
        rows: RangeInclusive<u32>,
        cols: RangeInclusive<u32>,
    ) -> Vec<((u32, u32), &Cell)> {
        self.cells
            .range((*rows.start(), *cols.start())..=(*rows.end(), *cols.end()))
            .map(|(k, v)| (*k, v))
            .collect()
    }

    /// Recomputes the tracked max bounds after a boundary removal. O(n) in
    /// present cells — only paid when the removed cell sat on the max row
    /// or column.
    fn recompute_bounds(&mut self) {
        self.max_row = self.cells.keys().next_back().map_or(0, |&(r, _)| r);
        self.max_col = self.cells.keys().fold(0, |acc, &(_, c)| acc.max(c));
    }
}

impl<'a> IntoIterator for &'a Sheet {
    type Item = ((u32, u32), &'a Cell);
    type IntoIter = alloc::boxed::Box<dyn Iterator<Item = ((u32, u32), &'a Cell)> + 'a>;

    /// Iterates present cells in row-major (deterministic) order — same as
    /// [`Sheet::iter`], so `for (coord, cell) in &sheet` works.
    fn into_iter(self) -> Self::IntoIter {
        alloc::boxed::Box::new(self.cells.iter().map(|(k, v)| (*k, v)))
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

    use super::Sheet;
    use crate::cell::{Cell, CellValue};
    use alloc::string::ToString;
    use alloc::{vec, vec::Vec};

    #[test]
    fn set_get_overwrite_and_absence() {
        let mut sheet = Sheet::new();
        assert!(sheet.is_empty());
        assert_eq!(sheet.len(), 0);
        assert_eq!(sheet.get(0, 0), None);

        sheet.set(3, 2, Cell::number(1.0));
        assert_eq!(sheet.get(3, 2), Some(&Cell::number(1.0)));
        assert_eq!(sheet.get(2, 3), None); // transposed: absent
        assert!(!sheet.is_empty());
        assert_eq!(sheet.len(), 1);

        sheet.set(3, 2, Cell::text("replaced"));
        assert_eq!(sheet.len(), 1); // overwrite, not insert
        assert_eq!(
            sheet.get(3, 2).map(|c| &c.value),
            Some(&CellValue::Text("replaced".to_string()))
        );
    }

    #[test]
    fn used_range_is_the_bounding_box() {
        let mut sheet = Sheet::new();
        assert_eq!(sheet.used_range(), None);

        // Deliberately non-corner shapes: (1,5) and (2,0).
        sheet.set(1, 5, Cell::number(1.0));
        sheet.set(2, 0, Cell::number(2.0));
        assert_eq!(sheet.used_range(), Some(((1, 0), (2, 5))));

        sheet.set(0, 7, Cell::number(3.0));
        assert_eq!(sheet.used_range(), Some(((0, 0), (2, 7))));
    }

    #[test]
    fn remove_shrinks_bounds_only_when_needed() {
        let mut sheet = Sheet::new();
        sheet.set(0, 0, Cell::number(1.0));
        sheet.set(5, 7, Cell::number(2.0));
        sheet.remove(0, 0); // interior for cols, boundary row? no: (0,0) row != max
        assert_eq!(sheet.used_range(), Some(((5, 7), (5, 7))));

        sheet.remove(5, 7); // boundary corner: bounds recompute to empty
        assert_eq!(sheet.used_range(), None);
        assert!(sheet.is_empty());

        // Removing an absent cell never panics nor corrupts bounds.
        sheet.remove(9, 9);
        assert_eq!(sheet.used_range(), None);

        sheet.set(2, 4, Cell::blank());
        sheet.remove(2, 4);
        assert_eq!(sheet.used_range(), None);
    }

    #[test]
    fn iteration_is_row_major_and_cells_in_bounds_correctly() {
        let mut sheet = Sheet::new();
        sheet.set(1, 0, Cell::number(1.0));
        sheet.set(0, 1, Cell::number(2.0));
        sheet.set(1, 1, Cell::number(3.0));

        let order: Vec<_> = sheet.iter().map(|(k, _)| *k).collect();
        assert_eq!(order, vec![(0, 1), (1, 0), (1, 1)]);

        let sliced: Vec<_> = sheet
            .cells_in(1..=1, 0..=1)
            .iter()
            .map(|(k, _)| *k)
            .collect();
        assert_eq!(sliced, vec![(1, 0), (1, 1)]);
        assert!(sheet.cells_in(5..=9, 5..=9).is_empty());
    }

    #[test]
    fn sheet_is_default_and_cloneable() {
        let sheet = Sheet::default();
        assert!(sheet.is_empty());
        let mut clone = sheet.clone();
        clone.set(0, 0, Cell::number(9.0));
        assert!(sheet.is_empty()); // deep copy
        assert_eq!(clone.used_range(), Some(((0, 0), (0, 0))));
    }
}
