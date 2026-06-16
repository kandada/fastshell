use crate::vfs::{self, Vfs};

#[derive(Debug, Clone)]
pub struct FileInfo {
    pub name: String,
    pub path: String,
    pub is_dir: bool,
    pub size: u64,
}

pub struct FsBridge {
    vfs: Vfs,
    cwd: String,
}

impl FsBridge {
    pub fn new(vfs: Vfs) -> Self {
        FsBridge {
            vfs,
            cwd: "/".to_string(),
        }
    }

    pub fn cwd(&self) -> &str {
        &self.cwd
    }

    pub fn set_cwd(&mut self, cwd: &str) {
        self.cwd = cwd.to_string();
    }

    pub fn read_file(&self, path: &str) -> vfs::Result<String> {
        self.vfs.read_to_string(path, &self.cwd)
    }

    pub fn read_file_bytes(&self, path: &str) -> vfs::Result<Vec<u8>> {
        self.vfs.read(path, &self.cwd)
    }

    pub fn write_file(&self, path: &str, content: &str) -> vfs::Result<()> {
        self.vfs.write(path, &self.cwd, content)
    }

    pub fn write_file_bytes(&self, path: &str, content: &[u8]) -> vfs::Result<()> {
        self.vfs.write_bytes(path, &self.cwd, content)
    }

    pub fn list_dir(&self, path: &str) -> vfs::Result<Vec<FileInfo>> {
        let entries = self.vfs.list_dir(path, &self.cwd)?;
        let base = if path.ends_with('/') {
            path.to_string()
        } else {
            format!("{}/", path)
        };

        Ok(entries
            .into_iter()
            .map(|e| FileInfo {
                path: format!("{}{}", base, e.name),
                name: e.name,
                is_dir: e.is_dir,
                size: e.size,
            })
            .collect())
    }

    pub fn create_dir(&self, path: &str) -> vfs::Result<()> {
        self.vfs.create_dir(path, &self.cwd)
    }

    pub fn create_dir_all(&self, path: &str) -> vfs::Result<()> {
        self.vfs.create_dir_all(path, &self.cwd)
    }

    pub fn remove_file(&self, path: &str) -> vfs::Result<()> {
        self.vfs.remove_file(path, &self.cwd)
    }

    pub fn remove_dir(&self, path: &str) -> vfs::Result<()> {
        self.vfs.remove_dir(path, &self.cwd)
    }

    pub fn remove_dir_all(&self, path: &str) -> vfs::Result<()> {
        self.vfs.remove_dir_all(path, &self.cwd)
    }

    pub fn exists(&self, path: &str) -> bool {
        self.vfs.exists(path, &self.cwd)
    }

    pub fn is_dir(&self, path: &str) -> bool {
        self.vfs.is_dir(path, &self.cwd)
    }

    pub fn is_file(&self, path: &str) -> bool {
        self.vfs.is_file(path, &self.cwd)
    }

    pub fn copy(&self, from: &str, to: &str) -> vfs::Result<()> {
        self.vfs.copy(from, to, &self.cwd)
    }

    pub fn rename(&self, from: &str, to: &str) -> vfs::Result<()> {
        self.vfs.rename(from, to, &self.cwd)
    }

    pub fn file_size(&self, path: &str) -> vfs::Result<u64> {
        self.vfs.metadata_len(path, &self.cwd)
    }

    pub fn to_vpath(&self, abs_path: &std::path::Path) -> String {
        self.vfs.to_vpath(abs_path)
    }

    pub fn vfs(&self) -> &Vfs {
        &self.vfs
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::vfs::Vfs;
    use std::fs;
    use std::sync::atomic::{AtomicUsize, Ordering};

    static TEST_COUNTER: AtomicUsize = AtomicUsize::new(0);

    fn setup_bridge() -> FsBridge {
        let n = TEST_COUNTER.fetch_add(1, Ordering::SeqCst);
        let dir = std::env::temp_dir()
            .join(format!("fastshell_bridge_fs_test_{}_{}", std::process::id(), n));
        let _ = fs::remove_dir_all(&dir);
        let vfs = Vfs::new(dir).unwrap();
        FsBridge::new(vfs)
    }

    #[test]
    fn test_read_write_file() {
        let bridge = setup_bridge();
        bridge.write_file("/test.txt", "hello").unwrap();
        let content = bridge.read_file("/test.txt").unwrap();
        assert_eq!(content, "hello");
    }

    #[test]
    fn test_list_dir() {
        let bridge = setup_bridge();
        bridge.create_dir("/mydir").unwrap();
        bridge.write_file("/mydir/a.txt", "a").unwrap();
        bridge.write_file("/mydir/b.txt", "b").unwrap();

        let files = bridge.list_dir("/mydir").unwrap();
        assert_eq!(files.len(), 2);
    }

    #[test]
    fn test_copy_rename() {
        let bridge = setup_bridge();
        bridge.write_file("/src.txt", "copy me").unwrap();
        bridge.copy("/src.txt", "/dst.txt").unwrap();
        assert_eq!(bridge.read_file("/dst.txt").unwrap(), "copy me");

        bridge.rename("/dst.txt", "/renamed.txt").unwrap();
        assert!(!bridge.exists("/dst.txt"));
        assert_eq!(bridge.read_file("/renamed.txt").unwrap(), "copy me");
    }
}
