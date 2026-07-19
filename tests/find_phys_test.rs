use fastshell::sdk::Fastshell;

fn setup() -> (Fastshell, std::path::PathBuf) {
    let dir = std::env::temp_dir().join(format!("fs_sdk_phys_{}_{}", std::process::id(), rand_suffix()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("projects/test1")).unwrap();
    std::fs::write(dir.join("projects/test1/init.md"), "hello").unwrap();
    std::fs::create_dir_all(dir.join("projects/test1/.aacode")).unwrap();
    let mut sdk = Fastshell::new();
    let cfg = fastshell::sdk::types::Config { sandbox_path: dir.to_string_lossy().to_string(), ..Default::default() };
    sdk.init(cfg).unwrap();
    (sdk, dir.canonicalize().unwrap())
}

fn rand_suffix() -> u128 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()
}

#[test]
fn sdk_find_full_physical_path_pipeline() {
    let (sdk, dir) = setup();
    let p = dir.join("projects/test1");
    let cmd = format!(
        "find '{}' -maxdepth 1 -type f -not -path '*/.git/*' -not -path '*/__pycache__/*' -not -path '*/.aacode/*' | sort",
        p.display()
    );
    let out = sdk.execute(&cmd);
    eprintln!("stdout={:?} stderr={:?} exit={}", out.stdout, out.stderr, out.exit_code);
    assert!(out.stdout.contains("init.md"), "expected init.md in output: {:?}", out);
}

#[test]
fn sdk_stat_and_cat_full_physical_path() {
    let (sdk, dir) = setup();
    let f = dir.join("projects/test1/init.md");
    let stat = sdk.execute(&format!("stat -c %s '{}'", f.display()));
    assert_eq!(stat.stdout.trim(), "5", "stat: {:?}", stat);
    let cat = sdk.execute(&format!("cat '{}'", f.display()));
    assert_eq!(cat.stdout, "hello", "cat: {:?}", cat);
}

#[test]
fn sdk_cd_full_physical_path_git_style() {
    let (sdk, dir) = setup();
    let p = dir.join("projects/test1");
    let out = sdk.execute(&format!("cd '{}' && pwd", p.display()));
    eprintln!("{:?}", out);
    assert_eq!(out.exit_code, 0, "cd should succeed: {:?}", out);
}
