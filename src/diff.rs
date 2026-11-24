//! Line diff between two texts.

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Line {
    Same(String),
    Added(String),
    Removed(String),
}

/// Above this many lines on either side the O(n·m) table is skipped and the diff
/// is "everything removed, everything added".
const MAX_LINES: usize = 5000;

/// Line-by-line diff turning `old` into `new`, in file order.
pub fn lines(old: &str, new: &str) -> Vec<Line> {
    let a: Vec<&str> = old.lines().collect();
    let b: Vec<&str> = new.lines().collect();
    if a.len() > MAX_LINES || b.len() > MAX_LINES {
        let removed = a.iter().map(|l| Line::Removed((*l).into()));
        let added = b.iter().map(|l| Line::Added((*l).into()));
        return removed.chain(added).collect();
    }
    // lcs[i][j]: length of the longest common subsequence of a[i..] and b[j..]
    let mut lcs = vec![vec![0u32; b.len() + 1]; a.len() + 1];
    for i in (0..a.len()).rev() {
        for j in (0..b.len()).rev() {
            lcs[i][j] = if a[i] == b[j] {
                lcs[i + 1][j + 1] + 1
            } else {
                lcs[i + 1][j].max(lcs[i][j + 1])
            };
        }
    }
    let mut out = Vec::with_capacity(a.len().max(b.len()));
    let (mut i, mut j) = (0, 0);
    while i < a.len() && j < b.len() {
        if a[i] == b[j] {
            out.push(Line::Same(a[i].into()));
            i += 1;
            j += 1;
        } else if lcs[i + 1][j] >= lcs[i][j + 1] {
            out.push(Line::Removed(a[i].into()));
            i += 1;
        } else {
            out.push(Line::Added(b[j].into()));
            j += 1;
        }
    }
    out.extend(a[i..].iter().map(|l| Line::Removed((*l).into())));
    out.extend(b[j..].iter().map(|l| Line::Added((*l).into())));
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn same(s: &str) -> Line {
        Line::Same(s.into())
    }
    fn add(s: &str) -> Line {
        Line::Added(s.into())
    }
    fn del(s: &str) -> Line {
        Line::Removed(s.into())
    }

    #[test]
    fn identical_texts_are_all_same() {
        assert_eq!(lines("a\nb", "a\nb"), [same("a"), same("b")]);
    }

    #[test]
    fn insert_delete_and_change() {
        assert_eq!(lines("a\nc", "a\nb\nc"), [same("a"), add("b"), same("c")]);
        assert_eq!(lines("a\nb\nc", "a\nc"), [same("a"), del("b"), same("c")]);
        assert_eq!(
            lines("a\nb\nc", "a\nx\nc"),
            [same("a"), del("b"), add("x"), same("c")]
        );
    }

    #[test]
    fn empty_sides() {
        assert!(lines("", "").is_empty());
        assert_eq!(lines("", "a"), [add("a")]);
        assert_eq!(lines("a", ""), [del("a")]);
    }

    #[test]
    fn huge_inputs_fall_back_to_replace_all() {
        let big = "x\n".repeat(MAX_LINES + 1);
        let d = lines(&big, "y");
        assert_eq!(d.len(), MAX_LINES + 2);
        assert_eq!(d.last(), Some(&add("y")));
    }
}
