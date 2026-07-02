// Copyright (c) 2025 xiefujin <490021684@qq.com>
// Licensed under Apache-2.0, see LICENSE file for full license terms.

use crate::shell::{CommandOutput, Shell};

#[derive(Clone, Copy, PartialEq)]
enum IgnoreWs {
    Changes,
    All,
}

impl Shell {
    pub fn cmd_diff(&self, args: &[&str]) -> CommandOutput {
        let mut unified = false;
        let mut recursive = false;
        let mut ignore_ws: Option<IgnoreWs> = None;
        let mut brief = false;
        let mut context_lines: usize = 3;
        let mut files: Vec<&str> = Vec::new();
        let mut i = 0;
        while i < args.len() {
            match args[i] {
                "-u" => unified = true,
                "-r" => recursive = true,
                "-b" => ignore_ws = Some(IgnoreWs::Changes),
                "-w" => ignore_ws = Some(IgnoreWs::All),
                "-q" => brief = true,
                "-U" => {
                    if i + 1 < args.len() {
                        if let Ok(n) = args[i + 1].parse::<usize>() {
                            context_lines = n;
                            i += 1;
                        }
                    }
                }
                a if a.starts_with("-U") && a.len() > 2 => {
                    if let Ok(n) = a[2..].parse::<usize>() {
                        context_lines = n;
                    }
                }
                _ if !args[i].starts_with('-') => files.push(args[i]),
                a if a.len() > 1 => {
                    for ch in a[1..].chars() {
                        match ch {
                            'u' => unified = true,
                            'r' => recursive = true,
                            'b' => ignore_ws = Some(IgnoreWs::Changes),
                            'w' => ignore_ws = Some(IgnoreWs::All),
                            'q' => brief = true,
                            _ => {}
                        }
                    }
                }
                _ => {}
            }
            i += 1;
        }

        if files.len() < 2 {
            return CommandOutput::error("diff: missing file operands\n".to_string(), 1);
        }

        let path1 = files[0];
        let path2 = files[1];

        let is_dir1 = self.vfs.is_dir(path1, &self.cwd);
        let is_dir2 = self.vfs.is_dir(path2, &self.cwd);

        if is_dir1 && is_dir2 {
            if recursive {
                return diff_directories(
                    self,
                    path1,
                    path2,
                    unified,
                    ignore_ws,
                    brief,
                    context_lines,
                );
            } else {
                return CommandOutput::error(
                    format!("diff: {} and {} are directories; use -r\n", path1, path2),
                    1,
                );
            }
        } else if is_dir1 {
            return CommandOutput::error(
                format!("diff: {} is a directory; {} is a file\n", path1, path2),
                1,
            );
        } else if is_dir2 {
            return CommandOutput::error(
                format!("diff: {} is a file; {} is a directory\n", path1, path2),
                1,
            );
        }

        let f1 = match self.vfs.read_to_string(path1, &self.cwd) {
            Ok(c) => c,
            Err(e) => return CommandOutput::error(format!("diff: {}: {}\n", path1, e), 1),
        };
        let f2 = match self.vfs.read_to_string(path2, &self.cwd) {
            Ok(c) => c,
            Err(e) => return CommandOutput::error(format!("diff: {}: {}\n", path2, e), 1),
        };

        let lines1: Vec<&str> = f1.lines().collect();
        let lines2: Vec<&str> = f2.lines().collect();

        if brief {
            if files_identical(&lines1, &lines2, ignore_ws) {
                return CommandOutput::success(String::new());
            } else {
                return CommandOutput {
                    stdout: format!("Files {} and {} differ\n", path1, path2),
                    stderr: String::new(),
                    exit_code: 1,
                };
            }
        }

        let output = if unified {
            compute_unified_diff(&lines1, &lines2, path1, path2, context_lines, ignore_ws)
        } else {
            compute_diff(&lines1, &lines2, ignore_ws)
        };

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

fn diff_directories(
    shell: &Shell,
    path1: &str,
    path2: &str,
    unified: bool,
    ignore_ws: Option<IgnoreWs>,
    brief: bool,
    context_lines: usize,
) -> CommandOutput {
    let mut output = String::new();
    let mut has_diff = false;

    let entries1 = match shell.vfs.list_dir(path1, &shell.cwd) {
        Ok(v) => v,
        Err(e) => return CommandOutput::error(format!("diff: {}: {}\n", path1, e), 1),
    };
    let entries2 = match shell.vfs.list_dir(path2, &shell.cwd) {
        Ok(v) => v,
        Err(e) => return CommandOutput::error(format!("diff: {}: {}\n", path2, e), 1),
    };

    let names1: Vec<String> = entries1.iter().map(|e| e.name.clone()).collect();
    let names2: Vec<String> = entries2.iter().map(|e| e.name.clone()).collect();

    let mut all_names: Vec<String> = names1.clone();
    for n in &names2 {
        if !all_names.contains(n) {
            all_names.push(n.clone());
        }
    }
    all_names.sort();

    for name in &all_names {
        let entry1 = entries1.iter().find(|e| &e.name == name);
        let entry2 = entries2.iter().find(|e| &e.name == name);

        let sub_path1 = format!("{}/{}", path1.trim_end_matches('/'), name);
        let sub_path2 = format!("{}/{}", path2.trim_end_matches('/'), name);

        match (entry1, entry2) {
            (Some(e1), Some(e2)) => {
                if e1.is_dir && e2.is_dir {
                    let sub_result = diff_directories(
                        shell,
                        &sub_path1,
                        &sub_path2,
                        unified,
                        ignore_ws,
                        brief,
                        context_lines,
                    );
                    if !sub_result.stdout.is_empty() {
                        output.push_str(&sub_result.stdout);
                    }
                    if sub_result.exit_code != 0 {
                        has_diff = true;
                    }
                } else if e1.is_dir || e2.is_dir {
                    output.push_str(&format!(
                        "File {} is a {} while file {} is a {}\n",
                        sub_path1,
                        if e1.is_dir {
                            "directory"
                        } else {
                            "regular file"
                        },
                        sub_path2,
                        if e2.is_dir {
                            "directory"
                        } else {
                            "regular file"
                        },
                    ));
                    has_diff = true;
                } else {
                    let f1_content = match shell.vfs.read_to_string(&sub_path1, &shell.cwd) {
                        Ok(c) => c,
                        Err(_) => {
                            output.push_str(&format!("diff: {}: cannot read\n", sub_path1));
                            has_diff = true;
                            continue;
                        }
                    };
                    let f2_content = match shell.vfs.read_to_string(&sub_path2, &shell.cwd) {
                        Ok(c) => c,
                        Err(_) => {
                            output.push_str(&format!("diff: {}: cannot read\n", sub_path2));
                            has_diff = true;
                            continue;
                        }
                    };
                    let lines1: Vec<&str> = f1_content.lines().collect();
                    let lines2: Vec<&str> = f2_content.lines().collect();

                    if brief {
                        if !files_identical(&lines1, &lines2, ignore_ws) {
                            output.push_str(&format!(
                                "Files {} and {} differ\n",
                                sub_path1, sub_path2
                            ));
                            has_diff = true;
                        }
                    } else {
                        let d = if unified {
                            compute_unified_diff(
                                &lines1,
                                &lines2,
                                &sub_path1,
                                &sub_path2,
                                context_lines,
                                ignore_ws,
                            )
                        } else {
                            compute_diff(&lines1, &lines2, ignore_ws)
                        };
                        if !d.is_empty() {
                            if unified {
                                output.push_str(&d);
                            } else {
                                output.push_str(&format!("diff {} {}\n", sub_path1, sub_path2));
                                output.push_str(&d);
                            }
                            has_diff = true;
                        }
                    }
                }
            }
            (Some(_), None) => {
                output.push_str(&format!("Only in {}: {}\n", path1, name));
                has_diff = true;
            }
            (None, Some(_)) => {
                output.push_str(&format!("Only in {}: {}\n", path2, name));
                has_diff = true;
            }
            (None, None) => {}
        }
    }

    if has_diff {
        CommandOutput {
            stdout: output,
            stderr: String::new(),
            exit_code: 1,
        }
    } else {
        CommandOutput::success(String::new())
    }
}

fn normalize_line(line: &str, mode: Option<IgnoreWs>) -> String {
    let mode = match mode {
        Some(m) => m,
        None => return line.to_string(),
    };
    let line = line.trim_end();
    match mode {
        IgnoreWs::All => line.chars().filter(|c| !c.is_whitespace()).collect(),
        IgnoreWs::Changes => {
            let mut result = String::with_capacity(line.len());
            let mut last_was_space = false;
            for ch in line.chars() {
                if ch.is_whitespace() {
                    if !last_was_space {
                        result.push(' ');
                        last_was_space = true;
                    }
                } else {
                    result.push(ch);
                    last_was_space = false;
                }
            }
            result.trim_end().to_string()
        }
    }
}

fn files_identical(a: &[&str], b: &[&str], ignore_ws: Option<IgnoreWs>) -> bool {
    if a.len() != b.len() {
        return false;
    }
    for (la, lb) in a.iter().zip(b.iter()) {
        if normalize_line(la, ignore_ws) != normalize_line(lb, ignore_ws) {
            return false;
        }
    }
    true
}

fn compute_diff(a: &[&str], b: &[&str], ignore_ws: Option<IgnoreWs>) -> String {
    let edits = build_edit_script(a, b, ignore_ws);
    let mut output = String::new();

    let mut chunks: Vec<Vec<&Edit>> = Vec::new();

    for edit in &edits {
        match edit {
            Edit::Keep(_) => {
                chunks.push(vec![edit]);
            }
            _ => {
                let mut chunk = vec![edit];
                while chunks
                    .last()
                    .map_or(false, |c| !matches!(c[0], Edit::Keep(_)))
                {
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

fn compute_unified_diff(
    a: &[&str],
    b: &[&str],
    label_a: &str,
    label_b: &str,
    context: usize,
    ignore_ws: Option<IgnoreWs>,
) -> String {
    let edits = build_edit_script(a, b, ignore_ws);

    let has_changes = edits.iter().any(|e| !matches!(e, Edit::Keep(_)));
    if !has_changes {
        return String::new();
    }

    let mut output = String::new();
    output.push_str(&format!("--- {}\n", label_a));
    output.push_str(&format!("+++ {}\n", label_b));

    // Convert edits to aligned line markers
    let mut markers: Vec<DiffMark> = Vec::new();
    for edit in &edits {
        match edit {
            Edit::Keep(i) => markers.push(DiffMark::Keep(*i)),
            Edit::Delete(i) => markers.push(DiffMark::Delete(*i)),
            Edit::Insert(j) => markers.push(DiffMark::Insert(*j)),
        }
    }

    // Find change runs and create hunks
    let n = markers.len();
    let mut i = 0;
    while i < n {
        // Skip keep lines
        while i < n && matches!(markers[i], DiffMark::Keep(_)) {
            i += 1;
        }
        if i >= n {
            break;
        }

        // Start of a change run
        let change_start = i;
        while i < n && !matches!(markers[i], DiffMark::Keep(_)) {
            i += 1;
        }
        let change_end = i;

        // Extend back for context
        let hunk_start = if change_start >= context {
            change_start - context
        } else {
            0
        };
        // Extend forward for context
        let hunk_end = if change_end + context <= n {
            change_end + context
        } else {
            n
        };

        // Trim leading context-only lines (no point)
        let mut actual_start = hunk_start;
        while actual_start < change_start
            && matches!(markers[actual_start], DiffMark::Keep(_))
            && actual_start + 1 < change_start
        {
            actual_start += 1;
        }
        // Keep at most `context` leading context lines
        if change_start > actual_start && change_start - actual_start > context + 1 {
            actual_start = change_start - context - 1;
            // Make sure we're at a keep line or start of change
            while actual_start < change_start && !matches!(markers[actual_start], DiffMark::Keep(_))
            {
                actual_start += 1;
            }
        }
        // For simplicity, just use hunk_start
        actual_start = hunk_start;

        let mut actual_end = hunk_end;
        while actual_end > change_end
            && matches!(markers[actual_end - 1], DiffMark::Keep(_))
            && actual_end - 1 > change_end
        {
            actual_end -= 1;
        }
        if actual_end < change_end + context && actual_end <= n {
            actual_end = change_end + context;
            if actual_end > n {
                actual_end = n;
            }
        }
        actual_end = actual_end.min(n);

        // Compute line ranges for old and new files
        let mut old_start = 0usize;
        let mut new_start = 0usize;
        let mut old_count = 0usize;
        let mut new_count = 0usize;

        for j in actual_start..actual_end {
            match &markers[j] {
                DiffMark::Keep(k) => {
                    if old_start == 0 {
                        old_start = *k;
                    }
                    if new_start == 0 {
                        new_start = *k;
                    }
                    old_count += 1;
                    new_count += 1;
                }
                DiffMark::Delete(k) => {
                    if old_start == 0 {
                        old_start = *k;
                    }
                    old_count += 1;
                }
                DiffMark::Insert(k) => {
                    if new_start == 0 {
                        new_start = *k;
                    }
                    new_count += 1;
                }
            }
        }

        // Fallback: scan for first old/new line numbers
        if old_start == 0 {
            for j in actual_start..actual_end {
                match &markers[j] {
                    DiffMark::Keep(k) | DiffMark::Delete(k) => {
                        old_start = *k;
                        break;
                    }
                    _ => {}
                }
            }
        }
        if new_start == 0 {
            for j in actual_start..actual_end {
                match &markers[j] {
                    DiffMark::Keep(k) | DiffMark::Insert(k) => {
                        new_start = *k;
                        break;
                    }
                    _ => {}
                }
            }
        }

        output.push_str(&format!(
            "@@ -{},{} +{},{} @@\n",
            old_start, old_count, new_start, new_count,
        ));

        for j in actual_start..actual_end {
            match &markers[j] {
                DiffMark::Keep(k) => {
                    output.push(' ');
                    output.push_str(a[k - 1]);
                    output.push('\n');
                }
                DiffMark::Delete(k) => {
                    output.push('-');
                    output.push_str(a[k - 1]);
                    output.push('\n');
                }
                DiffMark::Insert(k) => {
                    output.push('+');
                    output.push_str(b[k - 1]);
                    output.push('\n');
                }
            }
        }
    }

    output
}

enum DiffMark {
    Keep(usize),
    Delete(usize),
    Insert(usize),
}

enum Edit {
    Keep(usize),
    Delete(usize),
    Insert(usize),
}

fn build_edit_script(a: &[&str], b: &[&str], ignore_ws: Option<IgnoreWs>) -> Vec<Edit> {
    let n = a.len();
    let m = b.len();

    let norm_a: Vec<String> = a.iter().map(|l| normalize_line(l, ignore_ws)).collect();
    let norm_b: Vec<String> = b.iter().map(|l| normalize_line(l, ignore_ws)).collect();

    let lcs_table = build_lcs_table_str(&norm_a, &norm_b);
    let mut edits: Vec<Edit> = Vec::new();

    let mut i = n;
    let mut j = m;

    while i > 0 || j > 0 {
        if i > 0 && j > 0 && norm_a[i - 1] == norm_b[j - 1] {
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
    edits
}

fn build_lcs_table_str(a: &[String], b: &[String]) -> Vec<Vec<usize>> {
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
            Edit::Keep(i) | Edit::Delete(i) => {
                min = min.min(*i);
                max = max.max(*i);
            }
            Edit::Insert(j) => {
                min = min.min(*j);
                max = max.max(*j);
            }
        }
    }
    (min, max)
}

#[cfg(test)]
mod tests {
    use crate::shell::Shell;
    use crate::vfs::Vfs;
    use std::fs;
    use std::sync::atomic::{AtomicUsize, Ordering};

    static TEST_COUNTER: AtomicUsize = AtomicUsize::new(0);

    fn setup_vfs() -> Vfs {
        let n = TEST_COUNTER.fetch_add(1, Ordering::SeqCst);
        let dir =
            std::env::temp_dir().join(format!("fastshell_diff_test_{}_{}", std::process::id(), n));
        let _ = fs::remove_dir_all(&dir);
        Vfs::new(dir).unwrap()
    }

    fn mk_shell() -> Shell {
        Shell::new(setup_vfs())
    }

    #[test]
    fn test_diff() {
        let shell = mk_shell();
        shell
            .vfs
            .write("/a.txt", "", "line1\nline2\nline3\n")
            .unwrap();
        shell
            .vfs
            .write("/b.txt", "", "line1\nmodified\nline3\n")
            .unwrap();
        let out = shell.cmd_diff(&["/a.txt", "/b.txt"]);
        assert!(out.stdout.contains("modified"));
        assert_ne!(out.exit_code, 0);
    }

    #[test]
    fn test_diff_identical() {
        let shell = mk_shell();
        shell.vfs.write("/a.txt", "", "same\n").unwrap();
        shell.vfs.write("/b.txt", "", "same\n").unwrap();
        let out = shell.cmd_diff(&["/a.txt", "/b.txt"]);
        assert_eq!(out.exit_code, 0);
    }

    #[test]
    fn test_diff_unified() {
        let shell = mk_shell();
        shell
            .vfs
            .write("/a.txt", "", "line1\nline2\nline3\n")
            .unwrap();
        shell
            .vfs
            .write("/b.txt", "", "line1\nmodified\nline3\n")
            .unwrap();
        let out = shell.cmd_diff(&["-u", "/a.txt", "/b.txt"]);
        assert!(out.stdout.contains("--- /a.txt"));
        assert!(out.stdout.contains("+++ /b.txt"));
        assert!(out.stdout.contains("@@"));
        assert!(out.stdout.contains("-line2"));
        assert!(out.stdout.contains("+modified"));
        assert_ne!(out.exit_code, 0);
    }

    #[test]
    fn test_diff_unified_identical() {
        let shell = mk_shell();
        shell.vfs.write("/a.txt", "", "same\n").unwrap();
        shell.vfs.write("/b.txt", "", "same\n").unwrap();
        let out = shell.cmd_diff(&["-u", "/a.txt", "/b.txt"]);
        assert_eq!(out.exit_code, 0);
    }

    #[test]
    fn test_diff_brief() {
        let shell = mk_shell();
        shell.vfs.write("/a.txt", "", "hello\n").unwrap();
        shell.vfs.write("/b.txt", "", "world\n").unwrap();
        let out = shell.cmd_diff(&["-q", "/a.txt", "/b.txt"]);
        assert!(out.stdout.contains("differ"));
        assert_ne!(out.exit_code, 0);
    }

    #[test]
    fn test_diff_brief_identical() {
        let shell = mk_shell();
        shell.vfs.write("/a.txt", "", "same\n").unwrap();
        shell.vfs.write("/b.txt", "", "same\n").unwrap();
        let out = shell.cmd_diff(&["-q", "/a.txt", "/b.txt"]);
        assert_eq!(out.exit_code, 0);
    }

    #[test]
    fn test_diff_ignore_whitespace_changes() {
        let shell = mk_shell();
        shell
            .vfs
            .write("/a.txt", "", "hello world\nfoo  bar\n")
            .unwrap();
        shell
            .vfs
            .write("/b.txt", "", "hello world\nfoo    bar\n")
            .unwrap();
        let out = shell.cmd_diff(&["-b", "/a.txt", "/b.txt"]);
        assert_eq!(out.exit_code, 0);
    }

    #[test]
    fn test_diff_ignore_all_whitespace() {
        let shell = mk_shell();
        shell.vfs.write("/a.txt", "", "hello world\n").unwrap();
        shell.vfs.write("/b.txt", "", "helloworld\n").unwrap();
        let out = shell.cmd_diff(&["-w", "/a.txt", "/b.txt"]);
        assert_eq!(out.exit_code, 0);
    }

    #[test]
    fn test_diff_recursive_directories() {
        let shell = mk_shell();
        shell.cmd_mkdir(&["d1"]);
        shell.cmd_mkdir(&["d2"]);
        shell.vfs.write("/d1/a.txt", "", "hello\n").unwrap();
        shell.vfs.write("/d2/a.txt", "", "world\n").unwrap();
        let out = shell.cmd_diff(&["-r", "d1", "d2"]);
        assert!(
            out.stdout.contains("differ")
                || out.stdout.contains("-hello")
                || out.stdout.contains("+world")
                || out.stdout.contains("hello")
                || out.stdout.contains("world")
        );
        assert_ne!(out.exit_code, 0);
    }

    #[test]
    fn test_diff_recursive_only_in_one() {
        let shell = mk_shell();
        shell.cmd_mkdir(&["d1"]);
        shell.cmd_mkdir(&["d2"]);
        shell.vfs.write("/d1/only.txt", "", "data\n").unwrap();
        let out = shell.cmd_diff(&["-r", "d1", "d2"]);
        assert!(out.stdout.contains("Only in d1: only.txt"));
        assert_ne!(out.exit_code, 0);
    }

    #[test]
    fn test_diff_dirs_without_r_error() {
        let shell = mk_shell();
        shell.cmd_mkdir(&["d1"]);
        shell.cmd_mkdir(&["d2"]);
        let out = shell.cmd_diff(&["d1", "d2"]);
        assert_ne!(out.exit_code, 0);
        assert!(out.stderr.contains("directories"));
    }
}
