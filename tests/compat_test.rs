use std::fs;
use std::sync::atomic::{AtomicUsize, Ordering};
use fastshell::sdk::Fastshell;
use fastshell::sdk::types::Config;

static COUNTER: AtomicUsize = AtomicUsize::new(0);

fn setup() -> Fastshell {
    let n = COUNTER.fetch_add(1, Ordering::SeqCst);
    let dir = std::env::temp_dir().join(format!("fs_compat_{}_{}", std::process::id(), n));
    let _ = fs::remove_dir_all(&dir);
    let mut sdk = Fastshell::new();
    sdk.init(Config { sandbox_path: dir.to_string_lossy().to_string(), python_enabled: false, command_timeout_ms: 0 }).unwrap();

    sdk.write_file("hello.txt", "hello world\nfoo bar\nHELLO again\n").unwrap();
    sdk.write_file("nums.txt", "3\n1\n2\n2\n").unwrap();
    sdk.write_file("cols.txt", "a,b,c\nd,e,f\ng,h,i\n").unwrap();
    sdk.write_file("lorem.txt", "Lorem ipsum dolor sit amet\nconsectetur adipiscing elit\nsed do eiusmod tempor\n").unwrap();
    sdk
}

macro_rules! check {
    ($name:expr, $sdk:expr, $cmd:expr, $expect_ok:expr) => {
        let r = $sdk.execute($cmd);
        let sys = std::process::Command::new("sh").arg("-c")
            .arg(format!("cd {} && {}", $sdk.vfs_root(), $cmd))
            .output().map(|o| String::from_utf8_lossy(&o.stdout).to_string()).unwrap_or_default();
        let status = if r.is_success() == $expect_ok { "OK" } else { "EXIT" };
        let match_status = if r.stdout.trim() == sys.trim() { "OK" } else { "DIFF" };
        println!("[{}] {} | exit={}, sys_match={}", status, $name, match_status, $cmd);
        if match_status != "OK" {
            println!("  fastshell: {:?}", r.stdout);
            println!("  system:    {:?}", sys);
        }
        assert!(r.is_success() == $expect_ok, "{}: exit code mismatch for '{}'", $name, $cmd);
    };

    ($name:expr, $sdk:expr, $cmd:expr) => {
        check!($name, $sdk, $cmd, true);
    };
}

#[test]
fn test_compat_echo() {
    let sdk = setup();
    check!("echo_plain", sdk, "echo hello world");
    check!("echo_n", sdk, "echo -n hello");
}

#[test]
fn test_compat_cat() {
    let sdk = setup();
    check!("cat_file", sdk, "cat hello.txt");
    check!("cat_n", sdk, "cat -n hello.txt");
    check!("cat_multiple", sdk, "cat hello.txt nums.txt");
    check!("cat_empty", sdk, "cat /nonexistent.txt", false);
}

#[test]
fn test_compat_ls() {
    let sdk = setup();
    check!("ls_plain", sdk, "ls");
    check!("ls_all", sdk, "ls -a");
    check!("ls_long", sdk, "ls -l");
}

#[test]
fn test_compat_wc() {
    let sdk = setup();
    check!("wc_l", sdk, "wc -l hello.txt");
    check!("wc_w", sdk, "wc -w hello.txt");
    check!("wc_c", sdk, "wc -c hello.txt");
}

#[test]
fn test_compat_head_tail() {
    let sdk = setup();
    check!("head", sdk, "head -n 2 lorem.txt");
    check!("tail", sdk, "tail -n 2 lorem.txt");
}

#[test]
fn test_compat_grep() {
    let sdk = setup();
    check!("grep_match", sdk, "grep hello hello.txt");
    check!("grep_no_match", sdk, "grep zzz hello.txt", false);
    check!("grep_count", sdk, "grep -c hello hello.txt");
    check!("grep_invert", sdk, "grep -v hello hello.txt");
    check!("grep_ignore_case", sdk, "grep -i hello hello.txt");
}

#[test]
fn test_compat_sort_uniq() {
    let sdk = setup();
    check!("sort", sdk, "sort nums.txt");
    check!("sort_reverse", sdk, "sort -r nums.txt");
    check!("sort_numeric", sdk, "sort -n nums.txt");
    check!("uniq", sdk, "sort nums.txt | uniq");
}

#[test]
fn test_compat_cut() {
    let sdk = setup();
    check!("cut_field1", sdk, "cut -d, -f1 cols.txt");
    check!("cut_field2_3", sdk, "cut -d, -f2,3 cols.txt");
}

#[test]
fn test_compat_find() {
    let sdk = setup();
    check!("find_name_txt", sdk, "find . -name '*.txt'");
}

#[test]
fn test_compat_test() {
    let sdk = setup();
    check!("test_f_exists", sdk, "test -f hello.txt");
    check!("test_f_no", sdk, "test -f /nope.txt", false);
    check!("test_z_empty", sdk, "test -z ''");
    check!("test_z_no", sdk, "test -z 'abc'", false);
    check!("test_eq", sdk, "test 5 -eq 5");
    check!("test_eq_no", sdk, "test 5 -eq 6", false);
    check!("test_not", sdk, "test ! -f /nope.txt");
}

#[test]
fn test_compat_base64() {
    let sdk = setup();
    check!("base64_encode", sdk, "echo hello | base64");
}

#[test]
fn test_compat_du_df() {
    let sdk = setup();
    check!("du", sdk, "du -s .");
    check!("df", sdk, "df");
}

#[test]
fn test_compat_pipeline() {
    let sdk = setup();
    check!("pipe_wc", sdk, "cat hello.txt | wc -l");
    check!("pipe_grep_wc", sdk, "cat hello.txt | grep hello | wc -l");
    check!("pipe_sort_uniq", sdk, "cat nums.txt | sort -n | uniq");
}

#[test]
fn test_compat_sed() {
    let sdk = setup();
    check!("sed_subst", sdk, "sed 's/hello/hi/' hello.txt");
    check!("sed_delete", sdk, "sed '/foo/d' hello.txt");
}

#[test]
fn test_compat_tr() {
    let sdk = setup();
    check!("tr_translate", sdk, "echo hello | tr a-z A-Z");
    check!("tr_delete", sdk, "echo hello | tr -d l");
}

#[test]
fn test_compat_awk() {
    let sdk = setup();
    check!("awk_print1", sdk, "awk '{print $1}' hello.txt");
    check!("awk_nr", sdk, "awk 'NR>0 {print NR}' hello.txt");
}

#[test]
fn test_compat_sha256_base64() {
    let sdk = setup();
    check!("sha256sum", sdk, "echo hello | sha256sum | cut -d' ' -f1");
    check!("base64_decode", sdk, "echo aGVsbG8= | base64 -d");
}
