use criterion::{black_box, criterion_group, criterion_main, Criterion, Throughput};
use std::fs;
use std::sync::atomic::{AtomicUsize, Ordering};
use fastshell::sdk::Fastshell;
use fastshell::sdk::types::Config;

static COUNTER: AtomicUsize = AtomicUsize::new(0);

fn setup() -> Fastshell {
    let n = COUNTER.fetch_add(1, Ordering::SeqCst);
    let dir = std::env::temp_dir().join(format!("fs_bench_lat_{}_{}", std::process::id(), n));
    let _ = fs::remove_dir_all(&dir);
    let mut sdk = Fastshell::new();
    sdk.init(Config {
        sandbox_path: dir.to_string_lossy().to_string(),
        python_enabled: true,
        allow_subprocess: false,
        network_ask_permission: false,
        command_timeout_ms: 0,
    }).unwrap();
    for i in 0..100 {
        sdk.write_file(&format!("file_{}.txt", i), &format!("content {}", i)).unwrap();
    }
    sdk.execute("mkdir -p deep/nested/dir");
    sdk
}

fn bench_ls(c: &mut Criterion) {
    let mut group = c.benchmark_group("shell_latency");
    group.throughput(Throughput::Elements(1));
    let sdk = setup();

    group.bench_function("ls", |b| {
        b.iter(|| { let _ = black_box(sdk.execute(black_box("ls"))); })
    });

    group.bench_function("ls_la", |b| {
        b.iter(|| { let _ = black_box(sdk.execute(black_box("ls -la"))); })
    });

    group.bench_function("pwd", |b| {
        b.iter(|| { let _ = black_box(sdk.execute(black_box("pwd"))); })
    });

    group.bench_function("echo", |b| {
        b.iter(|| { let _ = black_box(sdk.execute(black_box("echo hello"))); })
    });

    group.bench_function("cat_small", |b| {
        b.iter(|| { let _ = black_box(sdk.execute(black_box("cat file_0.txt"))); })
    });

    group.bench_function("mkdir", |b| {
        b.iter(|| { let _ = black_box(sdk.execute(black_box("mkdir bench_dir"))); })
    });

    group.bench_function("grep_simple", |b| {
        b.iter(|| { let _ = black_box(sdk.execute(black_box("grep content file_0.txt"))); })
    });

    group.bench_function("wc", |b| {
        b.iter(|| { let _ = black_box(sdk.execute(black_box("wc -l file_0.txt"))); })
    });

    group.finish();
}

fn bench_pipeline_latency(c: &mut Criterion) {
    let mut group = c.benchmark_group("pipeline_latency");
    let sdk = setup();

    group.bench_function("echo_grep_wc", |b| {
        b.iter(|| { let _ = black_box(sdk.execute(black_box("echo hello world | grep hello | wc -w"))); })
    });

    group.bench_function("ls_grep", |b| {
        b.iter(|| { let _ = black_box(sdk.execute(black_box("ls | grep file"))); })
    });

    group.bench_function("cat_wc", |b| {
        b.iter(|| { let _ = black_box(sdk.execute(black_box("cat file_0.txt | wc -c"))); })
    });

    group.finish();
}

fn bench_filesystem_latency(c: &mut Criterion) {
    let mut group = c.benchmark_group("fs_latency");

    group.bench_function("read_file", |b| {
        let sdk = setup();
        b.iter(|| { let _ = black_box(sdk.read_file(black_box("file_0.txt"))); })
    });

    group.bench_function("write_file", |b| {
        let sdk = setup();
        b.iter(|| { let _ = black_box(sdk.write_file(black_box("bench_write.txt"), black_box("data"))); })
    });

    group.bench_function("exists_check", |b| {
        let sdk = setup();
        b.iter(|| { let _ = black_box(sdk.exists(black_box("file_0.txt"))); })
    });

    group.bench_function("list_dir", |b| {
        let sdk = setup();
        b.iter(|| { let _ = black_box(sdk.list_dir(black_box("/"))); })
    });

    group.finish();
}

fn bench_sdk_overhead(c: &mut Criterion) {
    let mut group = c.benchmark_group("sdk_overhead");

    group.bench_function("init_shutdown", |b| {
        b.iter(|| {
            let n = COUNTER.fetch_add(1, Ordering::SeqCst);
            let dir = std::env::temp_dir().join(format!("fs_bench_overhead_{}_{}", std::process::id(), n));
            let _ = fs::remove_dir_all(&dir);
            let mut sdk = Fastshell::new();
            sdk.init(Config {
                sandbox_path: dir.to_string_lossy().to_string(),
                python_enabled: false,
                allow_subprocess: false,
                network_ask_permission: false,
                command_timeout_ms: 0,
            }).unwrap();
            sdk.shutdown();
            let _ = fs::remove_dir_all(&dir);
        })
    });

    group.finish();
}

criterion_group!(benches, bench_ls, bench_pipeline_latency, bench_filesystem_latency, bench_sdk_overhead);
criterion_main!(benches);
