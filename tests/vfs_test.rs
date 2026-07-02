// Copyright (c) 2025 xiefujin <490021684@qq.com>
// Licensed under Apache-2.0, see LICENSE file for full license terms.

use std::fs;
use std::sync::atomic::{AtomicUsize, Ordering};

static COUNTER: AtomicUsize = AtomicUsize::new(0);

fn setup_dir() -> std::path::PathBuf {
    let n = COUNTER.fetch_add(1, Ordering::SeqCst);
    let dir = std::env::temp_dir().join(format!("fastshell_int_vfs_{}_{}", std::process::id(), n));
    let _ = fs::remove_dir_all(&dir);
    dir
}

#[test]
fn test_vfs_create_and_read() {
    let dir = setup_dir();
    let vfs = fastshell::vfs::Vfs::new(dir).unwrap();
    vfs.write("/hello.txt", "", "Hello").unwrap();
    assert_eq!(vfs.read_to_string("/hello.txt", "").unwrap(), "Hello");
}

#[test]
fn test_vfs_nested_dirs() {
    let dir = setup_dir();
    let vfs = fastshell::vfs::Vfs::new(dir).unwrap();
    vfs.create_dir_all("/a/b/c", "").unwrap();
    vfs.write("/a/b/c/data.txt", "", "nested").unwrap();
    assert!(vfs.exists("/a/b/c/data.txt", ""));
}

#[test]
fn test_vfs_path_escape_prevention() {
    let dir = setup_dir();
    let vfs = fastshell::vfs::Vfs::new(dir).unwrap();
    let result = vfs.resolve("../../etc/passwd", "/");
    assert!(result.is_err());
}

#[test]
fn test_vfs_copy_dir() {
    let dir = setup_dir();
    let vfs = fastshell::vfs::Vfs::new(dir).unwrap();
    vfs.create_dir("/src", "").unwrap();
    vfs.write("/src/a.txt", "", "a").unwrap();
    vfs.write("/src/b.txt", "", "b").unwrap();
    vfs.copy("/src", "/dst", "").unwrap();
    assert_eq!(vfs.read_to_string("/dst/a.txt", "").unwrap(), "a");
    assert_eq!(vfs.read_to_string("/dst/b.txt", "").unwrap(), "b");
}
