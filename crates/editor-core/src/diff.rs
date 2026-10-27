//! Line diff (Myers) between a file's committed or staged text and the
//! buffer, as the gutter's change markers.

use std::ops::Range;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChangeKind {
    Added,
    Modified,
    /// Lines removed just before `lines.start` (`lines` is empty).
    Deleted,
}

/// A run of changed lines, in the new text's zero-based lines.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LineChange {
    pub kind: ChangeKind,
    pub lines: Range<usize>,
}

/// Past this many differences the whole middle is marked modified rather
/// than diffed (time and memory grow with the square of it).
const MAX_EDITS: usize = 1000;

/// The changed lines of `new` compared with `old`.
pub fn line_changes(old: &str, new: &str) -> Vec<LineChange> {
    let a: Vec<&str> = old.lines().collect();
    let b: Vec<&str> = new.lines().collect();
    // Common ends are skipped first: most edits are local.
    let prefix = a.iter().zip(&b).take_while(|(x, y)| x == y).count();
    let suffix = a[prefix..].iter().rev().zip(b[prefix..].iter().rev()).take_while(|(x, y)| x == y).count();
    let (a_mid, b_mid) = (&a[prefix..a.len() - suffix], &b[prefix..b.len() - suffix]);

    let mut changes = Vec::new();
    match matches(a_mid, b_mid) {
        Some(pairs) => {
            // Between two matched lines (and at the ends): what was removed
            // from `a` and added in `b`.
            let (mut i, mut j) = (0, 0);
            for (x, y) in pairs.into_iter().chain([(a_mid.len(), b_mid.len())]) {
                push_hunk(&mut changes, x - i, prefix + j..prefix + y);
                (i, j) = (x + 1, y + 1);
            }
        }
        None => push_hunk(&mut changes, a_mid.len(), prefix..prefix + b_mid.len()),
    }
    changes
}

fn push_hunk(changes: &mut Vec<LineChange>, removed: usize, added: Range<usize>) {
    let kind = match (removed, added.is_empty()) {
        (0, true) => return,
        (0, false) => ChangeKind::Added,
        (_, true) => ChangeKind::Deleted,
        (_, false) => ChangeKind::Modified,
    };
    changes.push(LineChange { kind, lines: added });
}

/// Index pairs of the lines common to `a` and `b` (a longest common
/// subsequence), by Myers' algorithm; `None` past `MAX_EDITS`.
fn matches(a: &[&str], b: &[&str]) -> Option<Vec<(usize, usize)>> {
    let (n, m) = (a.len() as isize, b.len() as isize);
    let max = (n + m) as usize;
    if max == 0 {
        return Some(Vec::new());
    }
    let offset = max as isize;
    let mut v = vec![0isize; 2 * max + 2];
    // The frontier before each round, to walk back the path: only the
    // diagonals -d..=d round d reads (entry k + d), so it stays small.
    let mut trace: Vec<Vec<isize>> = Vec::new();
    let mut found = None;
    'search: for d in 0..=max.min(MAX_EDITS) as isize {
        trace.push(v[(offset - d) as usize..=(offset + d) as usize].to_vec());
        for k in (-d..=d).step_by(2) {
            let index = (k + offset) as usize;
            let mut x = if k == -d || (k != d && v[index - 1] < v[index + 1]) { v[index + 1] } else { v[index - 1] + 1 };
            let mut y = x - k;
            while x < n && y < m && a[x as usize] == b[y as usize] {
                x += 1;
                y += 1;
            }
            v[index] = x;
            if x >= n && y >= m {
                found = Some(d);
                break 'search;
            }
        }
    }
    let found = found?;

    // Walk back from the end, collecting the diagonal (matching) steps.
    let mut pairs = Vec::new();
    let (mut x, mut y) = (n, m);
    for d in (1..=found).rev() {
        let v = &trace[d as usize];
        let at = |k: isize| v[(k + d) as usize];
        let k = x - y;
        let previous_k = if k == -d || (k != d && at(k - 1) < at(k + 1)) { k + 1 } else { k - 1 };
        let previous_x = at(previous_k);
        let previous_y = previous_x - previous_k;
        while x > previous_x && y > previous_y {
            x -= 1;
            y -= 1;
            pairs.push((x as usize, y as usize));
        }
        (x, y) = (previous_x, previous_y);
    }
    // The snake of the first round (d = 0): matching lines from the start.
    while x > 0 && y > 0 {
        x -= 1;
        y -= 1;
        pairs.push((x as usize, y as usize));
    }
    pairs.reverse();
    Some(pairs)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn change(kind: ChangeKind, lines: Range<usize>) -> LineChange {
        LineChange { kind, lines }
    }

    #[test]
    fn finds_added_modified_and_deleted_lines() {
        let old = "a\nb\nc\nd\ne\n";
        assert_eq!(line_changes(old, old), []);
        assert_eq!(line_changes(old, "a\nb\nX\nc\nd\ne\n"), [change(ChangeKind::Added, 2..3)]);
        assert_eq!(line_changes(old, "a\nB\nc\nd\ne\n"), [change(ChangeKind::Modified, 1..2)]);
        assert_eq!(line_changes(old, "a\nb\nd\ne\n"), [change(ChangeKind::Deleted, 2..2)]);
        assert_eq!(line_changes(old, "b\nc\nd\ne\n"), [change(ChangeKind::Deleted, 0..0)]);
        assert_eq!(line_changes(old, "a\nb\nc\nd\ne\nf\n"), [change(ChangeKind::Added, 5..6)]);
        assert_eq!(line_changes("", "x\ny\n"), [change(ChangeKind::Added, 0..2)]);
    }

    #[test]
    fn separate_edits_stay_separate() {
        let old = "1\n2\n3\n4\n5\n6\n7\n8\n";
        let new = "1\nTWO\n3\n4\n5\nnew\n6\n7\n";
        assert_eq!(
            line_changes(old, new),
            [change(ChangeKind::Modified, 1..2), change(ChangeKind::Added, 5..6), change(ChangeKind::Deleted, 8..8)]
        );
    }

    #[test]
    fn large_files_stay_quick() {
        let old: String = (0..20_000).map(|i| format!("line {i}\n")).collect();
        // Edits near both ends: the middle between them is long but alike.
        let new = old.replacen("line 10\n", "changed\n", 1).replacen("line 19990\n", "", 1);
        assert_eq!(
            line_changes(&old, &new),
            [change(ChangeKind::Modified, 10..11), change(ChangeKind::Deleted, 19990..19990)]
        );
        // Everything different: past the limit, one modified block.
        let other: String = (0..3000).map(|i| format!("other {i}\n")).collect();
        assert_eq!(line_changes(&old[..30_000], &other), [change(ChangeKind::Modified, 0..3000)]);
    }

    #[test]
    fn matches_form_a_longest_common_subsequence() {
        let a = ["x", "a", "b", "c", "y"];
        let b = ["a", "z", "b", "c"];
        let pairs = matches(&a, &b).unwrap();
        assert_eq!(pairs, [(1, 0), (2, 2), (3, 3)]);
    }
}
