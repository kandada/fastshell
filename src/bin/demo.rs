use fastshell::sdk::Fastshell;
use fastshell::sdk::types::Config;

fn main() {
    let mut sdk = Fastshell::new();

    let tmp = std::env::temp_dir().join("fastshell_demo");
    let _ = std::fs::remove_dir_all(&tmp);

    let config = Config {
        sandbox_path: tmp.to_string_lossy().to_string(),
        python_enabled: true,
        ..Default::default()
    };
    sdk.init(config).unwrap();

    println!("======== fastshell Demo ========");
    let info = sdk.get_info();
    println!("Version : {}", info.version);
    println!("Platform: {}", info.platform);
    println!("Python  : {}", if info.python_available { "available" } else { "not available" });
    println!("Sandbox : {}", info.sandbox_path);
    println!();

    println!("--- [1] ls (empty) ---");
    let r = sdk.execute("ls");
    println!("stdout: {}", r.stdout.trim());
    println!("exit: {}", r.exit_code);

    println!();
    println!("--- [2] mkdir work && cd work ---");
    sdk.execute("mkdir work");
    sdk.execute("cd work");

    println!();
    println!("--- [3] pwd ---");
    let r = sdk.execute("pwd");
    println!("stdout: {}", r.stdout.trim());

    println!();
    println!("--- [4] touch file && echo hello ---");
    sdk.execute("touch hello.py");
    sdk.execute("touch readme.md");

    let r = sdk.execute("ls -l");
    println!("stdout:\n{}", r.stdout);

    println!();
    println!("--- [5] cat (create file via vfs) ---");
    // write through shell's vfs
    let _ = sdk.execute("touch data.txt");
    let r = sdk.execute("cat data.txt");
    println!("stdout: {}", r.stdout.trim());
    println!("exit: {}", r.exit_code);

    println!();
    println!("--- [6] find ---");
    sdk.execute("mkdir subdir");
    sdk.execute("touch subdir/nested.txt");
    let r = sdk.execute("find .");
    println!("stdout:\n{}", r.stdout);

    println!();
    println!("--- [7] grep ---");
    sdk.execute("cd /");
    // Write some content
    let _ = sdk.execute("touch log.txt");
    let r = sdk.execute("grep hello log.txt");
    println!("stdout: {}", r.stdout.trim());
    println!("exit: {}", r.exit_code);

    println!();
    println!("--- [8] Python ---");
    let r = sdk.execute_python("print('Hello from Python inside fastshell!')");
    println!("stdout: {}", r.stdout.trim());
    println!("exit: {}", r.exit_code);

    let r = sdk.execute_python("import sys; print(f'Python {sys.version}')");
    println!("stdout: {}", r.stdout.trim());

    let r = sdk.execute_python("print(sum(range(1, 101)))");
    println!("1+2+...+100 = {}", r.stdout.trim());

    println!();
    println!("--- [9] cd /work && rm cleanup ---");
    sdk.execute("cd /work");
    sdk.execute("rm hello.py");
    sdk.execute("rm readme.md");
    let r = sdk.execute("ls");
    println!("remaining: {}", r.stdout.trim());

    println!();
    println!("--- [10] error handling ---");
    let r = sdk.execute("cat nonexistent.txt");
    println!("stdout: {}", r.stdout.trim());
    println!("stderr: {}", r.stderr.trim());
    println!("exit: {}", r.exit_code);

    let r = sdk.execute("nonexistent_cmd_xyz arg1 arg2");
    println!("stdout: {}", r.stdout.trim());
    println!("stderr: {}", r.stderr.trim());
    println!("exit: {}", r.exit_code);

    println!();
    println!("======== Demo Complete ========");

    let _ = std::fs::remove_dir_all(&tmp);
}
