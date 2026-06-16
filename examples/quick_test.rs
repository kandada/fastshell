use fastshell::sdk::Fastshell;
use fastshell::sdk::types::Config;

fn main() {
    let mut s = Fastshell::new();
    let d = std::env::temp_dir().join("fs_quick");
    let _ = std::fs::remove_dir_all(&d);
    s.init(Config {
        sandbox_path: d.to_string_lossy().into(),
        python_enabled: false,
        ..Default::default()
    })
    .unwrap();

    let cmds = &[
        "whoami",
        "ps",
        "uname -a",
        "tar --version",
        "which ls",
        "echo 'pipe test'",
    ];
    for cmd in cmds {
        let r = s.execute(cmd);
        let first_line = r.stdout.lines().next().unwrap_or("(empty)");
        println!(
            "[{}] exit={} | {}",
            cmd, r.exit_code, first_line
        );
    }
}
