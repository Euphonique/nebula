//! Line-based diff (Myers' algorithm) for change markers and comparisons.

use std::hash::{Hash, Hasher};
use std::ops::Range;

/// A region where the old and new text differ (line indices).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Hunk {
    pub old: Range<usize>,
    pub new: Range<usize>,
}

/// Above this many differing lines the middle part counts as one change.
const MAX_EDITS: usize = 2000;

fn hash(line: &str) -> u64 {
    let mut h = std::collections::hash_map::DefaultHasher::new();
    line.hash(&mut h);
    h.finish()
}

/// Differences between two texts given as lines.
pub fn hunks(old: &[&str], new: &[&str]) -> Vec<Hunk> {
    let a: Vec<u64> = old.iter().map(|l| hash(l)).collect();
    let b: Vec<u64> = new.iter().map(|l| hash(l)).collect();
    let prefix = a.iter().zip(&b).take_while(|(x, y)| x == y).count();
    let room = a.len().min(b.len()) - prefix;
    let suffix = a.iter().rev().zip(b.iter().rev()).take(room).take_while(|(x, y)| x == y).count();
    let (a_mid, b_mid) = (&a[prefix..a.len() - suffix], &b[prefix..b.len() - suffix]);
    let ops = myers(a_mid, b_mid).unwrap_or_else(|| {
        let mut ops = vec![Op::Delete; a_mid.len()];
        ops.extend(std::iter::repeat_n(Op::Insert, b_mid.len()));
        ops
    });
    // Group consecutive non-equal operations.
    let mut out = Vec::new();
    let (mut x, mut y) = (prefix, prefix);
    let mut current: Option<Hunk> = None;
    for op in ops {
        match op {
            Op::Equal => {
                out.extend(current.take());
                x += 1;
                y += 1;
            }
            Op::Delete => {
                current.get_or_insert(Hunk { old: x..x, new: y..y }).old.end = x + 1;
                x += 1;
            }
            Op::Insert => {
                current.get_or_insert(Hunk { old: x..x, new: y..y }).new.end = y + 1;
                y += 1;
            }
        }
    }
    out.extend(current);
    out
}

/// Splits text into lines without line breaks (CRLF and LF).
pub fn lines(text: &str) -> Vec<&str> {
    let text = text.strip_suffix('\n').unwrap_or(text);
    if text.is_empty() {
        return Vec::new();
    }
    text.split('\n').map(|l| l.strip_suffix('\r').unwrap_or(l)).collect()
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Op {
    Equal,
    Delete,
    Insert,
}

/// Shortest edit script; `None` if it needs more than `MAX_EDITS` edits.
fn myers(a: &[u64], b: &[u64]) -> Option<Vec<Op>> {
    let (n, m) = (a.len() as isize, b.len() as isize);
    let max = (n + m) as usize;
    // trace[d][k + d] = furthest x on diagonal k after d edits.
    let mut trace: Vec<Vec<isize>> = Vec::new();
    let mut done = false;
    for d in 0..=max.min(MAX_EDITS) as isize {
        let mut v = vec![0isize; (2 * d + 1) as usize];
        let prev = trace.last();
        let get = |k: isize| -> isize {
            let p = prev.unwrap();
            p[(k + d - 1) as usize]
        };
        for k in (-d..=d).step_by(2) {
            let mut x = if d == 0 {
                0
            } else if k == -d || (k != d && get(k - 1) < get(k + 1)) {
                get(k + 1)
            } else {
                get(k - 1) + 1
            };
            let mut y = x - k;
            while x < n && y < m && a[x as usize] == b[y as usize] {
                x += 1;
                y += 1;
            }
            v[(k + d) as usize] = x;
            if x >= n && y >= m {
                done = true;
                break;
            }
        }
        trace.push(v);
        if done {
            break;
        }
    }
    if !done {
        return None;
    }
    // Walk back from the end.
    let mut ops = Vec::new();
    let (mut x, mut y) = (n, m);
    for d in (1..trace.len() as isize).rev() {
        let prev = &trace[(d - 1) as usize];
        let get = |k: isize| prev[(k + d - 1) as usize];
        let k = x - y;
        let prev_k = if k == -d || (k != d && get(k - 1) < get(k + 1)) { k + 1 } else { k - 1 };
        let prev_x = get(prev_k);
        let prev_y = prev_x - prev_k;
        while x > prev_x && y > prev_y {
            ops.push(Op::Equal);
            x -= 1;
            y -= 1;
        }
        ops.push(if prev_k == k + 1 { Op::Insert } else { Op::Delete });
        x = prev_x;
        y = prev_y;
    }
    while x > 0 && y > 0 {
        ops.push(Op::Equal);
        x -= 1;
        y -= 1;
    }
    ops.reverse();
    Some(ops)
}

/// One row of a unified diff for display.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DiffLine {
    /// Unchanged line with its number in the old and new text.
    Context(usize, usize, String),
    Removed(usize, String),
    Added(usize, String),
    /// Unchanged lines left out.
    Skip(usize),
}

/// Unified diff rows with `context` unchanged lines around each change.
pub fn unified(old: &str, new: &str, context: usize) -> Vec<DiffLine> {
    let (a, b) = (lines(old), lines(new));
    let hunks = hunks(&a, &b);
    let mut out = Vec::new();
    let mut x = 0; // next old line not yet shown
    for (i, h) in hunks.iter().enumerate() {
        let start = h.old.start.saturating_sub(context).max(x);
        if start > x {
            out.push(DiffLine::Skip(start - x));
        }
        let offset = h.new.start as isize - h.old.start as isize;
        for l in start..h.old.start {
            out.push(DiffLine::Context(l, (l as isize + offset) as usize, a[l].to_string()));
        }
        for l in h.old.clone() {
            out.push(DiffLine::Removed(l, a[l].to_string()));
        }
        for l in h.new.clone() {
            out.push(DiffLine::Added(l, b[l].to_string()));
        }
        let next = hunks.get(i + 1).map_or(a.len(), |n| n.old.start);
        let end = (h.old.end + context).min(next).min(a.len());
        let offset = h.new.end as isize - h.old.end as isize;
        for l in h.old.end..end {
            out.push(DiffLine::Context(l, (l as isize + offset) as usize, a[l].to_string()));
        }
        x = end;
    }
    if !hunks.is_empty() && x < a.len() {
        out.push(DiffLine::Skip(a.len() - x));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_changes() {
        let old = ["a", "b", "c", "d", "e"];
        let new = ["a", "B", "c", "e", "f"];
        let h = hunks(&old, &new);
        assert_eq!(
            h,
            vec![
                Hunk { old: 1..2, new: 1..2 },
                Hunk { old: 3..4, new: 3..3 },
                Hunk { old: 5..5, new: 4..5 },
            ]
        );
        assert!(hunks(&old, &old).is_empty());
        assert_eq!(hunks(&[], &["x"]), vec![Hunk { old: 0..0, new: 0..1 }]);
    }

    #[test]
    fn unified_rows() {
        let rows = unified("1\n2\n3\n4\n5\n6\n7\n8\n", "1\n2\n3\n4\nfünf\n6\n7\n8\n", 1);
        assert_eq!(
            rows,
            vec![
                DiffLine::Skip(3),
                DiffLine::Context(3, 3, "4".into()),
                DiffLine::Removed(4, "5".into()),
                DiffLine::Added(4, "fünf".into()),
                DiffLine::Context(5, 5, "6".into()),
                DiffLine::Skip(2),
            ]
        );
    }
}
