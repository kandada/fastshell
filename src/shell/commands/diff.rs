use crate::shell::{Shell, CommandOutput};

impl Shell {
    pub fn cmd_diff(&self, args: &[&str]) -> CommandOutput {
        let files: Vec<&str> = args
            .iter()
            .filter(|a| !a.starts_with('-'))
            .copied()
            .collect();

        if files.len() < 2 {
            return CommandOutput::error("diff: missing file operands\n".to_string(), 1);
        }

        let f1 = match self.vfs.read_to_string(files[0], &self.cwd) {
            Ok(c) => c,
            Err(e) => return CommandOutput::error(format!("diff: {}: {}\n", files[0], e), 1),
        };
        let f2 = match self.vfs.read_to_string(files[1], &self.cwd) {
            Ok(c) => c,
            Err(e) => return CommandOutput::error(format!("diff: {}: {}\n", files[1], e), 1),
        };

        let lines1: Vec<&str> = f1.lines().collect();
        let lines2: Vec<&str> = f2.lines().collect();

        let output = compute_diff(&lines1, &lines2);

        if output.is_empty() {
            CommandOutput::success(String::new())
        } else {
            CommandOutput {
                stdout: output,
                stderr: String::new(),
                exit_code: 1,
            }
        }
    }
}

fn compute_diff(a: &[&str], b: &[&str]) -> String {
    let n = a.len();
    let m = b.len();
    let lcs_table = build_lcs_table(a, b);
    let mut output = String::new();

    let mut i = n;
    let mut j = m;
    let mut edits: Vec<Edit> = Vec::new();

    while i > 0 || j > 0 {
        if i > 0 && j > 0 && a[i - 1] == b[j - 1] {
            edits.push(Edit::Keep(i));
            i -= 1;
            j -= 1;
        } else if j > 0 && (i == 0 || lcs_table[i][j - 1] >= lcs_table[i - 1][j]) {
            edits.push(Edit::Insert(j));
            j -= 1;
        } else if i > 0 {
            edits.push(Edit::Delete(i));
            i -= 1;
        } else {
            break;
        }
    }

    edits.reverse();
    let mut chunks: Vec<Vec<&Edit>> = Vec::new();

    for edit in &edits {
        match edit {
            Edit::Keep(_) => {
                chunks.push(vec![edit]);
            }
            _ => {
                let mut chunk = vec![edit];
                while chunks.last().map_or(false, |c| {
                    !matches!(c[0], Edit::Keep(_))
                }) {
                    let prev = chunks.pop().unwrap();
                    chunk.splice(0..0, prev.into_iter());
                }
                if chunk.iter().any(|e| matches!(e, Edit::Delete(..)))
                    && chunk.iter().any(|e| matches!(e, Edit::Insert(..)))
                {
                    format_change_chunk(&mut output, &chunk, a, b);
                } else if chunk.iter().any(|e| matches!(e, Edit::Delete(..))) {
                    format_delete_chunk(&mut output, &chunk, a, b);
                } else {
                    format_insert_chunk(&mut output, &chunk, a, b);
                }
            }
        }
    }

    output
}

enum Edit {
    Keep(usize),
    Delete(usize),
    Insert(usize),
}

fn build_lcs_table(a: &[&str], b: &[&str]) -> Vec<Vec<usize>> {
    let n = a.len();
    let m = b.len();
    let mut dp = vec![vec![0usize; m + 1]; n + 1];

    for i in 1..=n {
        for j in 1..=m {
            if a[i - 1] == b[j - 1] {
                dp[i][j] = dp[i - 1][j - 1] + 1;
            } else {
                dp[i][j] = dp[i - 1][j].max(dp[i][j - 1]);
            }
        }
    }

    dp
}

fn format_change_chunk(output: &mut String, chunk: &[&Edit], a: &[&str], b: &[&str]) {
    let (start_a, start_b) = chunk_start(chunk);
    let (end_a, _) = chunk_end(chunk);
    let count_a = end_a - start_a + 1;
    let (_, end_b) = chunk_range(chunk);

    if count_a == 1 && (end_b - start_b + 1) == 1 {
        output.push_str(&format!("{}c{}\n", start_a, start_b));
    } else if count_a == 0 {
        if end_b - start_b == 0 {
            output.push_str(&format!("{}a{}\n", start_a, start_b));
        } else {
            output.push_str(&format!("{}a{},{}\n", start_a, start_b, end_b));
        }
    } else if end_b - start_b + 1 == 0 {
        output.push_str(&format!("{},{}d{}\n", start_a, end_a, start_b));
    } else {
        output.push_str(&format!("{},{}c{},{}\n", start_a, end_a, start_b, end_b));
    }

    for edit in chunk {
        match edit {
            Edit::Delete(i) => {
                output.push_str(&format!("< {}\n", a[i - 1]));
            }
            Edit::Insert(j) => {
                output.push_str(&format!("> {}\n", b[j - 1]));
            }
            _ => {}
        }
    }
}

fn format_delete_chunk(output: &mut String, chunk: &[&Edit], a: &[&str], _b: &[&str]) {
    let (start, end) = chunk_range(chunk);
    if start == end {
        output.push_str(&format!("{}d{}\n", start, end));
    } else {
        output.push_str(&format!("{},{}d{}\n", start, end, end));
    }
    for edit in chunk {
        if let Edit::Delete(i) = edit {
            output.push_str(&format!("< {}\n", a[i - 1]));
        }
    }
}

fn format_insert_chunk(output: &mut String, chunk: &[&Edit], _a: &[&str], b: &[&str]) {
    let (start, end) = chunk_range(chunk);
    let base_line = start.saturating_sub(1);
    if start == end {
        output.push_str(&format!("{}a{}\n", base_line, start));
    } else {
        output.push_str(&format!("{}a{},{}\n", base_line, start, end));
    }
    for edit in chunk {
        if let Edit::Insert(j) = edit {
            output.push_str(&format!("> {}\n", b[j - 1]));
        }
    }
}

fn chunk_start(chunk: &[&Edit]) -> (usize, usize) {
    let mut min_a = usize::MAX;
    let mut min_b = usize::MAX;
    for edit in chunk {
        match edit {
            Edit::Keep(i) | Edit::Delete(i) => min_a = min_a.min(*i),
            Edit::Insert(j) => min_b = min_b.min(*j),
        }
    }
    (min_a, min_b)
}

fn chunk_end(chunk: &[&Edit]) -> (usize, usize) {
    let mut max_a = 0;
    let mut max_b = 0;
    for edit in chunk {
        match edit {
            Edit::Keep(i) | Edit::Delete(i) => max_a = max_a.max(*i),
            Edit::Insert(j) => max_b = max_b.max(*j),
        }
    }
    (max_a, max_b)
}

fn chunk_range(chunk: &[&Edit]) -> (usize, usize) {
    let mut min = usize::MAX;
    let mut max = 0;
    for edit in chunk {
        match edit {
            Edit::Keep(i) | Edit::Delete(i) => { min = min.min(*i); max = max.max(*i); }
            Edit::Insert(j) => { min = min.min(*j); max = max.max(*j); }
        }
    }
    (min, max)
}
