//! Streaming edit-argument previews. Trim common edges before budgeting the LCS matrix.

use std::fmt::Write as _;

use serde_json::Value;

use super::DiffView;

pub(super) fn build_edit_arg_diff(args: &Value) -> Option<DiffView> {
    let parsed;
    let args = if let Some(raw) = args.as_str() {
        parsed = serde_json::from_str(raw).ok()?;
        &parsed
    } else {
        args
    };
    let path = args.get("path")?.as_str()?.to_owned();
    let old = args
        .get("old_str")
        .or_else(|| args.get("oldText"))
        .and_then(Value::as_str)
        .unwrap_or_default();
    let new = args
        .get("new_str")
        .or_else(|| args.get("newText"))
        .and_then(Value::as_str)
        .unwrap_or_default();
    if old.is_empty() && new.is_empty() {
        return None;
    }
    let old: Vec<_> = old.split_inclusive('\n').collect();
    let new: Vec<_> = new.split_inclusive('\n').collect();
    let head = old.iter().zip(&new).take_while(|(a, b)| a == b).count();
    let tail = old[head..]
        .iter()
        .rev()
        .zip(new[head..].iter().rev())
        .take_while(|(a, b)| a == b)
        .count();

    let mut unified = String::new();
    let mut added_lines = 0;
    let mut removed_lines = 0;
    let mut emit = |marker, line: &str| {
        added_lines += usize::from(marker == '+');
        removed_lines += usize::from(marker == '-');
        let display = line
            .strip_suffix("\r\n")
            .or_else(|| line.strip_suffix('\n'))
            .unwrap_or(line);
        writeln!(unified, "{marker}{display}").expect("writing to a String");
        if marker != ' '
            && !line.ends_with('\n')
            && old.last().is_some_and(|l| l.ends_with('\n'))
                != new.last().is_some_and(|l| l.ends_with('\n'))
        {
            unified.push_str("\\ No newline at end of file\n");
        }
    };
    for line in &old[..head] {
        emit(' ', line);
    }
    compare_middle(
        &old[head..old.len() - tail],
        &new[head..new.len() - tail],
        &mut emit,
    );
    for line in &old[old.len() - tail..] {
        emit(' ', line);
    }
    unified.pop();
    Some(DiffView {
        path,
        unified,
        added_lines,
        removed_lines,
    })
}

fn compare_middle(old: &[&str], new: &[&str], mut emit: impl FnMut(char, &str)) {
    // Limit both work per streaming update and temporary memory (512 KiB).
    const MAX_CELLS: usize = 256 * 1024;
    let stride = new.len() + 1;
    let cells = (old.len() + 1).saturating_mul(stride);
    if old.is_empty() || new.is_empty() || cells > MAX_CELLS {
        for line in old {
            emit('-', line);
        }
        for line in new {
            emit('+', line);
        }
        return;
    }

    let mut lcs = vec![0u16; cells];
    for i in (0..old.len()).rev() {
        for j in (0..new.len()).rev() {
            let k = i * stride + j;
            lcs[k] = if old[i] == new[j] {
                1 + lcs[k + stride + 1]
            } else {
                lcs[k + stride].max(lcs[k + 1])
            };
        }
    }
    let (mut i, mut j) = (0, 0);
    while i < old.len() || j < new.len() {
        let k = i * stride + j;
        if i < old.len() && j < new.len() && old[i] == new[j] {
            emit(' ', old[i]);
            i += 1;
            j += 1;
        } else if i < old.len() && (j == new.len() || lcs[k + stride] >= lcs[k + 1]) {
            emit('-', old[i]);
            i += 1;
        } else {
            emit('+', new[j]);
            j += 1;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::build_edit_arg_diff;
    use serde_json::json;

    #[test]
    fn edit_preview_preserves_context_between_independent_changes() {
        let diff = build_edit_arg_diff(&json!({
            "path":"app.rs", "oldText":"prefix\nold\nunchanged\nold2\nsuffix",
            "newText":"prefix\nnew\nunchanged\nnew2\nsuffix",
        }))
        .unwrap();
        assert_eq!((diff.added_lines, diff.removed_lines), (2, 2));
        assert_eq!(
            diff.unified,
            " prefix\n-old\n+new\n unchanged\n-old2\n+new2\n suffix"
        );
    }

    #[test]
    fn oversized_replacements_preserve_shared_edges_and_count_edits() {
        let diff = build_edit_arg_diff(&json!({
            "path":"data", "old_str": format!("first\n{}last", "a\n".repeat(2000)),
            "new_str": format!("first\n{}last", "b\n".repeat(2000)),
        }))
        .unwrap();
        assert_eq!((diff.added_lines, diff.removed_lines), (2000, 2000));
        assert!(diff.unified.starts_with(" first\n-a\n"));
        assert!(diff.unified.ends_with("\n+b\n last"));
    }
}
