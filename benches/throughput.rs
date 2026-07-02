// Copyright (c) 2025 xiefujin <490021684@qq.com>
// Licensed under Apache-2.0, see LICENSE file for full license terms.

use criterion::{black_box, criterion_group, criterion_main, Criterion, Throughput};
use fastshell::sdk::types::Config;
use fastshell::sdk::Fastshell;
use std::fs;
use std::sync::atomic::{AtomicUsize, Ordering};

static COUNTER: AtomicUsize = AtomicUsize::new(0);

fn setup() -> Fastshell {
    let n = COUNTER.fetch_add(1, Ordering::SeqCst);
    let dir = std::env::temp_dir().join(format!("fs_bench_tp_{}_{}", std::process::id(), n));
    let _ = fs::remove_dir_all(&dir);
    let mut sdk = Fastshell::new();
    sdk.init(Config {
        sandbox_path: dir.to_string_lossy().to_string(),
        python_enabled: true,
        allow_subprocess: false,
        network_ask_permission: false,
        command_timeout_ms: 0,
    })
    .unwrap();
    sdk
}

fn bench_bulk_file_ops(c: &mut Criterion) {
    let mut group = c.benchmark_group("bulk_file_ops");

    group.bench_function("create_100_files", |b| {
        b.iter(|| {
            let sdk = setup();
            for i in 0..100 {
                let _ = black_box(sdk.write_file(&format!("f_{}.txt", i), "content"));
            }
        })
    });

    group.bench_function("cat_1mb_file_write_read", |b| {
        let sdk = setup();
        let data = "Line of text for testing\n".repeat(10000);
        sdk.write_file("big.txt", &data).unwrap();
        b.iter(|| {
            let _ = black_box(sdk.read_file(black_box("big.txt")));
        })
    });

    group.finish();
}

fn bench_grep_large(c: &mut Criterion) {
    let mut group = c.benchmark_group("grep_large");
    group.throughput(Throughput::Bytes(100_000));

    let sdk = setup();
    let lines: Vec<String> = (0..5000).map(|i| format!("line {} data", i)).collect();
    sdk.write_file("large.txt", &lines.join("\n")).unwrap();
    sdk.write_file("medium.txt", &lines[..1000].join("\n"))
        .unwrap();

    group.bench_function("grep_5000_lines", |b| {
        b.iter(|| {
            let _ = black_box(sdk.execute(black_box("grep line large.txt")));
        })
    });

    group.bench_function("grep_1000_lines", |b| {
        b.iter(|| {
            let _ = black_box(sdk.execute(black_box("grep line medium.txt")));
        })
    });

    group.finish();
}

fn bench_sort_awk(c: &mut Criterion) {
    let mut group = c.benchmark_group("sort_awk");

    let sdk = setup();
    let nums: Vec<String> = (0..1000)
        .map(|_| format!("{}", rand::random::<u32>()))
        .collect();
    sdk.write_file("nums.txt", &nums.join("\n")).unwrap();

    group.bench_function("sort_1000", |b| {
        b.iter(|| {
            let _ = black_box(sdk.execute(black_box("sort -n nums.txt")));
        })
    });

    group.finish();
}

fn bench_sed_diff(c: &mut Criterion) {
    let mut group = c.benchmark_group("sed_diff");

    let sdk = setup();
    let lines: Vec<String> = (0..500).map(|i| format!("line {} original", i)).collect();
    sdk.write_file("text1.txt", &lines.join("\n")).unwrap();
    let lines2: Vec<String> = (0..500).map(|i| format!("line {} modified", i)).collect();
    sdk.write_file("text2.txt", &lines2.join("\n")).unwrap();

    group.bench_function("sed_replace_500", |b| {
        b.iter(|| {
            let _ = black_box(sdk.execute(black_box("sed 's/original/replaced/g' text1.txt")));
        })
    });

    group.bench_function("diff_500", |b| {
        b.iter(|| {
            let _ = black_box(sdk.execute(black_box("diff text1.txt text2.txt")));
        })
    });

    group.finish();
}

criterion_group!(
    benches,
    bench_bulk_file_ops,
    bench_grep_large,
    bench_sort_awk,
    bench_sed_diff
);
criterion_main!(benches);
