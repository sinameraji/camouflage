//! Build `DiffLine`s from before/after text or from a unified diff string.

use crate::blocks::{DiffKind, DiffLine};

/// Line diff of two texts with `context` unchanged lines around each change.
/// Uses an LCS table, so it's quadratic: inputs over ~4k lines per side fall
/// back to showing the new text as added.
pub fn diff_texts(before: &str, after: &str, context: usize) -> Vec<DiffLine> {
    let a: Vec<&str> = before.lines().collect();
    let b: Vec<&str> = after.lines().collect();
    if a.len().saturating_mul(b.len()) > 16_000_000 {
        return b
            .iter()
            .enumerate()
            .map(|(i, t)| DiffLine { line_no: Some(i as u32 + 1), kind: DiffKind::Add, text: t.to_string() })
            .collect();
    }
    let (n, m) = (a.len(), b.len());
    let mut lcs = vec![vec![0u32; m + 1]; n + 1];
    for i in (0..n).rev() {
        for j in (0..m).rev() {
            lcs[i][j] = if a[i] == b[j] { lcs[i + 1][j + 1] + 1 } else { lcs[i + 1][j].max(lcs[i][j + 1]) };
        }
    }
    // Full edit script with line numbers (old numbers for removals, new
    // numbers for additions and context).
    let mut script: Vec<DiffLine> = Vec::new();
    let (mut i, mut j) = (0, 0);
    while i < n || j < m {
        if i < n && j < m && a[i] == b[j] {
            script.push(DiffLine { line_no: Some(j as u32 + 1), kind: DiffKind::Context, text: b[j].to_string() });
            i += 1;
            j += 1;
        } else if i < n && (j == m || lcs[i + 1][j] >= lcs[i][j + 1]) {
            script.push(DiffLine { line_no: Some(i as u32 + 1), kind: DiffKind::Del, text: a[i].to_string() });
            i += 1;
        } else {
            script.push(DiffLine { line_no: Some(j as u32 + 1), kind: DiffKind::Add, text: b[j].to_string() });
            j += 1;
        }
    }
    // Keep changes plus `context` lines around them.
    let changed: Vec<usize> = script.iter().enumerate().filter(|(_, d)| d.kind != DiffKind::Context).map(|(k, _)| k).collect();
    script
        .into_iter()
        .enumerate()
        .filter(|(k, d)| d.kind != DiffKind::Context || changed.iter().any(|c| c.abs_diff(*k) <= context))
        .map(|(_, d)| d)
        .collect()
}

/// Parse a unified diff (`@@ -a,b +c,d @@` hunks). File headers are skipped.
pub fn parse_unified(diff: &str) -> Vec<DiffLine> {
    let mut out = Vec::new();
    let (mut old, mut new) = (0u32, 0u32);
    for line in diff.lines() {
        if line.starts_with("@@") {
            let nums: Vec<u32> = line
                .split(|c: char| !c.is_ascii_digit())
                .filter(|s| !s.is_empty())
                .filter_map(|s| s.parse().ok())
                .collect();
            old = *nums.first().unwrap_or(&1);
            new = *nums.get(2).or(nums.get(1)).unwrap_or(&1);
            continue;
        }
        if line.starts_with("+++") || line.starts_with("---") || line.starts_with("diff ") || line.starts_with("index ") {
            continue;
        }
        if let Some(t) = line.strip_prefix('+') {
            out.push(DiffLine { line_no: Some(new), kind: DiffKind::Add, text: t.to_string() });
            new += 1;
        } else if let Some(t) = line.strip_prefix('-') {
            out.push(DiffLine { line_no: Some(old), kind: DiffKind::Del, text: t.to_string() });
            old += 1;
        } else {
            let t = line.strip_prefix(' ').unwrap_or(line);
            out.push(DiffLine { line_no: Some(new), kind: DiffKind::Context, text: t.to_string() });
            old += 1;
            new += 1;
        }
    }
    out
}

pub fn count_changes(lines: &[DiffLine]) -> (usize, usize) {
    let add = lines.iter().filter(|d| d.kind == DiffKind::Add).count();
    let del = lines.iter().filter(|d| d.kind == DiffKind::Del).count();
    (add, del)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn show(lines: &[DiffLine]) -> Vec<String> {
        lines
            .iter()
            .map(|d| {
                let s = match d.kind {
                    DiffKind::Add => '+',
                    DiffKind::Del => '-',
                    DiffKind::Context => ' ',
                };
                format!("{}{s}{}", d.line_no.unwrap_or(0), d.text)
            })
            .collect()
    }

    #[test]
    fn diffs_texts_with_context() {
        let before = "a\nb\nc\nd\ne\nf\ng";
        let after = "a\nb\nc\nD\ne\nf\ng";
        assert_eq!(show(&diff_texts(before, after, 1)), vec!["3 c", "4-d", "4+D", "5 e"]);
    }

    #[test]
    fn parses_unified_hunks() {
        let d = "--- a/x.ts\n+++ b/x.ts\n@@ -42,3 +42,3 @@\n return token\n-// TODO\n+rotate()\n }";
        assert_eq!(show(&parse_unified(d)), vec!["42 return token", "43-// TODO", "43+rotate()", "44 }"]);
    }
}
