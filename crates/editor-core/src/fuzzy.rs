//! Fuzzy matching for quick open and the command palette: the needle's
//! characters must appear in order; matches at word starts and runs of
//! consecutive characters score higher, gaps cost a little.

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FuzzyMatch {
    pub score: i64,
    /// Char indices in the haystack of each matched character.
    pub positions: Vec<usize>,
}

const MATCH: i64 = 16;
const CONSECUTIVE: i64 = 12;
const START_OF_TEXT: i64 = 10;
const AFTER_SEPARATOR: i64 = 9;
const CAMEL_CASE: i64 = 7;
const SAME_CASE: i64 = 1;

/// Best match of `needle` in `haystack` (case-insensitive), if any.
pub fn fuzzy_match(needle: &str, haystack: &str) -> Option<FuzzyMatch> {
    let needle: Vec<char> = needle.chars().filter(|c| !c.is_whitespace()).collect();
    if needle.is_empty() {
        return Some(FuzzyMatch { score: 0, positions: Vec::new() });
    }
    let text: Vec<char> = haystack.chars().collect();
    let (m, n) = (needle.len(), text.len());
    if m > n {
        return None;
    }
    let lower = |c: char| c.to_lowercase().next().unwrap_or(c);
    let needle_lower: Vec<char> = needle.iter().copied().map(lower).collect();
    let text_lower: Vec<char> = text.iter().copied().map(lower).collect();

    let bonus = |j: usize| -> i64 {
        if j == 0 {
            return START_OF_TEXT;
        }
        let (prev, here) = (text[j - 1], text[j]);
        if matches!(prev, '/' | '\\' | '_' | '-' | '.' | ' ' | ':') {
            AFTER_SEPARATOR
        } else if prev.is_lowercase() && here.is_uppercase() {
            CAMEL_CASE
        } else {
            0
        }
    };
    let gain = |i: usize, j: usize| MATCH + bonus(j) + if text[j] == needle[i] { SAME_CASE } else { 0 };

    const NONE: i64 = i64::MIN / 4;
    // score[i][j]: best score with needle[i] matched at text[j].
    let mut score = vec![vec![NONE; n]; m];
    let mut back = vec![vec![usize::MAX; n]; m];
    for j in 0..n {
        if text_lower[j] == needle_lower[0] {
            // A late first match costs a little, so "player" beats "my_player".
            score[0][j] = gain(0, j) - (j as i64 / 4);
        }
    }
    for i in 1..m {
        // Best previous match at least two chars back, with the gap cost
        // folded in: score + k, so subtracting j later gives the penalty.
        let (mut far_best, mut far_from) = (NONE, usize::MAX);
        for j in i..n {
            if j >= 2 {
                let k = j - 2;
                if score[i - 1][k] > NONE && score[i - 1][k] + k as i64 > far_best {
                    far_best = score[i - 1][k] + k as i64;
                    far_from = k;
                }
            }
            if text_lower[j] != needle_lower[i] {
                continue;
            }
            let adjacent = score[i - 1][j - 1];
            let adjacent = if adjacent > NONE { adjacent + CONSECUTIVE } else { NONE };
            let far = if far_best > NONE { far_best - (j as i64 - 1) } else { NONE };
            let (best, from) = if adjacent >= far { (adjacent, j - 1) } else { (far, far_from) };
            if best > NONE {
                score[i][j] = best + gain(i, j);
                back[i][j] = from;
            }
        }
    }

    let (mut j, best) = (0..n)
        .map(|j| (j, score[m - 1][j]))
        .max_by_key(|&(j, s)| (s, std::cmp::Reverse(j)))?;
    if best <= NONE {
        return None;
    }
    let mut positions = vec![0; m];
    for i in (0..m).rev() {
        positions[i] = j;
        j = back[i][j];
    }
    Some(FuzzyMatch { score: best, positions })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn score(needle: &str, haystack: &str) -> i64 {
        fuzzy_match(needle, haystack).map_or(i64::MIN, |m| m.score)
    }

    #[test]
    fn requires_characters_in_order() {
        assert!(fuzzy_match("plgd", "player.gd").is_some());
        assert!(fuzzy_match("dgp", "player.gd").is_none());
        assert!(fuzzy_match("xyz", "player.gd").is_none());
        assert_eq!(fuzzy_match("", "anything").unwrap().score, 0);
    }

    #[test]
    fn prefers_word_starts_and_runs() {
        assert!(score("pl", "player.gd") > score("pl", "apple.gd"));
        assert!(score("mp", "main_player.gd") > score("mp", "ample.gd"));
        assert!(score("player", "player.gd") > score("player", "scenes/my_player_old.gd"));
        assert!(score("tc", "ToggleComment") > score("tc", "tactic"));
    }

    #[test]
    fn reports_positions_of_the_best_alignment() {
        let m = fuzzy_match("pgd", "player.gd").unwrap();
        assert_eq!(m.positions, vec![0, 7, 8]);
        let m = fuzzy_match("sp", "scenes/player.gd").unwrap();
        assert_eq!(m.positions, vec![0, 7]);
        let m = fuzzy_match("É", "élan").unwrap();
        assert_eq!(m.positions, vec![0]);
    }
}
