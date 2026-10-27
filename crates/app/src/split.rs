//! The two sides of the split editor: which side each tab is on, which
//! tab each side shows, and which side has focus.

/// Split state besides the tabs' sides.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Split {
    /// The tab with keyboard focus (on the focused side).
    pub active: Option<usize>,
    /// The tab each side shows.
    pub shown: [Option<usize>; 2],
    /// 0 left, 1 right.
    pub focused: usize,
}

impl Split {
    /// Makes the state consistent with the tabs' sides (`groups[i]` is
    /// tab `i`'s side): the active tab decides the focused side, each side
    /// shows one of its own tabs (the last one if its choice is gone), and a
    /// right side left alone becomes the only (left) side.
    pub fn sync(&mut self, groups: &mut [usize]) {
        let len = groups.len();
        if self.active.is_some_and(|a| a >= len) {
            self.active = None;
        }
        if let Some(active) = self.active {
            self.focused = groups[active];
            self.shown[self.focused] = Some(active);
        }
        let has = |groups: &[usize], side: usize| groups.contains(&side);
        if has(groups, 1) && !has(groups, 0) {
            groups.iter_mut().for_each(|g| *g = 0);
            self.shown = [self.shown[1], None];
            self.focused = 0;
        }
        for side in 0..2 {
            let valid = self.shown[side].filter(|&i| i < len && groups[i] == side);
            self.shown[side] = valid.or_else(|| groups.iter().rposition(|&g| g == side));
        }
        if !has(groups, 1) {
            self.focused = 0;
        }
        if self.active.is_none_or(|a| groups[a] != self.focused) {
            self.active = self.shown[self.focused];
        }
    }
}

/// Narrowest a side gets.
const MIN_SIDE_WIDTH: f32 = 150.0;

/// Width of the left side for a split `ratio` of `width` (the two sides
/// together), leaving each side at least `MIN_SIDE_WIDTH` when it fits.
pub fn left_width(ratio: f32, width: f32) -> f32 {
    if width < 2.0 * MIN_SIDE_WIDTH {
        return (width / 2.0).round();
    }
    (ratio * width).round().clamp(MIN_SIDE_WIDTH, width - MIN_SIDE_WIDTH)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sides_keep_a_minimum_width() {
        assert_eq!(left_width(0.5, 1000.0), 500.0);
        assert_eq!(left_width(0.3, 1000.0), 300.0);
        assert_eq!(left_width(0.0, 1000.0), 150.0);
        assert_eq!(left_width(1.0, 1000.0), 850.0);
        // Too narrow for both minimums: half each.
        assert_eq!(left_width(0.1, 200.0), 100.0);
    }

    #[test]
    fn active_tab_decides_the_focused_side() {
        let mut groups = [0, 0, 1];
        let mut split = Split { active: Some(2), shown: [Some(0), None], focused: 0 };
        split.sync(&mut groups);
        assert_eq!(split, Split { active: Some(2), shown: [Some(0), Some(2)], focused: 1 });
    }

    #[test]
    fn a_side_whose_tab_is_gone_shows_another() {
        // Tab 1 (shown on the left) was closed: the left shows its last tab.
        let mut groups = [0, 0, 1];
        let mut split = Split { active: Some(2), shown: [None, Some(2)], focused: 1 };
        split.sync(&mut groups);
        assert_eq!(split.shown, [Some(1), Some(2)]);
    }

    #[test]
    fn emptied_sides_close_the_split() {
        // The last right tab moved left: back to one side, focus there.
        let mut groups = [0, 0];
        let mut split = Split { active: Some(1), shown: [Some(0), Some(1)], focused: 1 };
        split.sync(&mut groups);
        assert_eq!(split, Split { active: Some(1), shown: [Some(1), None], focused: 0 });

        // Only right tabs left: they become the left side.
        let mut groups = [1, 1];
        let mut split = Split { active: Some(0), shown: [None, Some(0)], focused: 1 };
        split.sync(&mut groups);
        assert_eq!(groups, [0, 0]);
        assert_eq!(split, Split { active: Some(0), shown: [Some(0), None], focused: 0 });

        let mut none: [usize; 0] = [];
        let mut split = Split { active: Some(3), shown: [Some(3), Some(4)], focused: 1 };
        split.sync(&mut none);
        assert_eq!(split, Split::default());
    }
}
