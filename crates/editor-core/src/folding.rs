//! Code folding: which line ranges can fold (indentation blocks and
//! `#region` / `#endregion`), and the mapping between buffer lines and the
//! rows shown once some are hidden.

/// Foldable ranges `(header, last)`: the header line stays visible, lines
/// `header + 1 ..= last` hide. Sorted by header.
///
/// An indentation block is a line followed by more-indented lines (blank
/// lines inside count, trailing ones don't). A `#region` comment folds down
/// to its matching `#endregion`, inclusive.
pub fn fold_ranges<'a>(lines: impl IntoIterator<Item = &'a str>, tab_width: usize) -> Vec<(usize, usize)> {
    let mut ranges = Vec::new();
    // Open indentation blocks: (header line, its indent).
    let mut open: Vec<(usize, usize)> = Vec::new();
    let mut regions: Vec<usize> = Vec::new();
    let mut last_text = None;
    for (index, line) in lines.into_iter().enumerate() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        let indent = indent_width(line, tab_width);
        while open.last().is_some_and(|&(_, d)| d >= indent) {
            let (header, _) = open.pop().unwrap_or_default();
            close(&mut ranges, header, last_text);
        }
        if trimmed.starts_with("#region") {
            regions.push(index);
        } else if trimmed.starts_with("#endregion") {
            if let Some(header) = regions.pop() {
                ranges.push((header, index));
            }
        }
        open.push((index, indent));
        last_text = Some(index);
    }
    while let Some((header, _)) = open.pop() {
        close(&mut ranges, header, last_text);
    }
    ranges.sort_unstable();
    ranges.dedup_by_key(|r| r.0);
    ranges
}

fn close(ranges: &mut Vec<(usize, usize)>, header: usize, last_text: Option<usize>) {
    if let Some(last) = last_text.filter(|&last| last > header) {
        ranges.push((header, last));
    }
}

fn indent_width(line: &str, tab_width: usize) -> usize {
    let mut width = 0;
    for c in line.chars() {
        match c {
            ' ' => width += 1,
            '\t' => width += tab_width - width % tab_width,
            _ => break,
        }
    }
    width
}

/// Lines hidden by folds, as sorted, merged inclusive ranges.
pub fn hidden_ranges(folded: &[(usize, usize)]) -> Vec<(usize, usize)> {
    let mut hidden: Vec<(usize, usize)> = folded.iter().map(|&(header, last)| (header + 1, last)).collect();
    hidden.sort_unstable();
    let mut merged: Vec<(usize, usize)> = Vec::new();
    for (start, end) in hidden {
        match merged.last_mut() {
            Some(last) if start <= last.1 + 1 => last.1 = last.1.max(end),
            _ => merged.push((start, end)),
        }
    }
    merged
}

/// Maps buffer lines to the rows shown on screen, skipping hidden lines.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct LineMap {
    /// Sorted, merged inclusive ranges of hidden lines.
    hidden: Vec<(usize, usize)>,
    lines: usize,
}

impl LineMap {
    pub fn new(hidden: Vec<(usize, usize)>, lines: usize) -> Self {
        Self { hidden, lines }
    }

    /// Number of rows shown.
    pub fn rows(&self) -> usize {
        self.lines - self.hidden.iter().map(|(s, e)| e - s + 1).sum::<usize>()
    }

    pub fn is_hidden(&self, line: usize) -> bool {
        self.range_of(line).is_some()
    }

    /// The hidden range containing `line`, if any.
    pub fn range_of(&self, line: usize) -> Option<(usize, usize)> {
        let index = self.hidden.partition_point(|&(start, _)| start <= line);
        index.checked_sub(1).map(|i| self.hidden[i]).filter(|&(_, end)| line <= end)
    }

    /// The row a line is shown on; a hidden line maps to the row of the
    /// line its fold hangs from.
    pub fn row_of(&self, line: usize) -> usize {
        let line = match self.range_of(line) {
            Some((start, _)) => start.saturating_sub(1),
            None => line,
        };
        let hidden_before: usize = self.hidden.iter().take_while(|&&(s, _)| s < line).map(|(s, e)| e - s + 1).sum();
        line - hidden_before
    }

    /// The line shown on `row` (the last line for rows past the end).
    pub fn line_at(&self, row: usize) -> usize {
        let mut line = row;
        for &(start, end) in &self.hidden {
            if start <= line {
                line += end - start + 1;
            } else {
                break;
            }
        }
        line.min(self.lines.saturating_sub(1))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const CODE: &str = "extends Node\n\nfunc a():\n\tif x:\n\t\tpass\n\n\tpass\n\n#region Helpers\nfunc b():\n\tpass\n#endregion\n";

    #[test]
    fn indentation_blocks_and_regions() {
        let ranges = fold_ranges(CODE.lines(), 4);
        // func a (lines 2..6, trailing blank excluded), its if (3..4),
        // the region (8..11), func b (9..10).
        assert_eq!(ranges, [(2, 6), (3, 4), (8, 11), (9, 10)]);
        assert!(fold_ranges("a\nb\n".lines(), 4).is_empty());
    }

    #[test]
    fn hidden_ranges_merge_nested_folds() {
        assert_eq!(hidden_ranges(&[(2, 6), (3, 4)]), [(3, 6)]);
        assert_eq!(hidden_ranges(&[(8, 11), (2, 6)]), [(3, 6), (9, 11)]);
    }

    #[test]
    fn line_map_round_trips_visible_lines() {
        // 12 lines, 3..=6 and 9..=11 hidden: rows show 0 1 2 7 8.
        let map = LineMap::new(vec![(3, 6), (9, 11)], 12);
        assert_eq!(map.rows(), 5);
        let shown: Vec<usize> = (0..map.rows()).map(|row| map.line_at(row)).collect();
        assert_eq!(shown, [0, 1, 2, 7, 8]);
        for (row, &line) in shown.iter().enumerate() {
            assert_eq!(map.row_of(line), row);
        }
        // A hidden line sits on its fold's header row.
        assert_eq!(map.row_of(5), 2);
        assert!(map.is_hidden(10) && !map.is_hidden(8));
        assert_eq!(map.line_at(99), 11);
    }
}
