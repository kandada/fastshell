use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

#[derive(Debug)]
pub enum VfsError {
    Io(io::Error),
    PathEscape(PathBuf),
    NotFound(String),
    NotADirectory(String),
    AlreadyExists(String),
}

impl std::fmt::Display for VfsError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            VfsError::Io(e) => write!(f, "IO error: {}", e),
            VfsError::PathEscape(p) => write!(f, "Path escapes sandbox: {}", p.display()),
            VfsError::NotFound(p) => write!(f, "Not found: {}", p),
            VfsError::NotADirectory(p) => write!(f, "Not a directory: {}", p),
            VfsError::AlreadyExists(p) => write!(f, "Already exists: {}", p),
        }
    }
}

impl std::error::Error for VfsError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            VfsError::Io(e) => Some(e),
            _ => None,
        }
    }
}

impl From<io::Error> for VfsError {
    fn from(e: io::Error) -> Self {
        VfsError::Io(e)
    }
}

pub type Result<T> = std::result::Result<T, VfsError>;

#[derive(Debug, Clone)]
pub struct DirEntry {
    pub name: String,
    pub is_dir: bool,
    pub size: u64,
    pub modified: Option<SystemTime>,
}

#[derive(Debug, Clone)]
pub struct Vfs {
    root: PathBuf,
}

impl Vfs {
    pub fn new(root: PathBuf) -> Result<Self> {
        if !root.exists() {
            fs::create_dir_all(&root)?;
        }
        let root = root.canonicalize()?;
        Ok(Vfs { root })
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn resolve(&self, path: &str, cwd: &str) -> Result<PathBuf> {
        let candidate = if path.starts_with('/') {
            self.root.join(path.trim_start_matches('/'))
        } else {
            let base = if cwd.starts_with('/') {
                self.root.join(cwd.trim_start_matches('/'))
            } else if cwd.is_empty() {
                self.root.clone()
            } else {
                self.root.join(cwd)
            };
            base.join(path)
        };

        let resolved = normalize_path(&candidate);

        if !resolved.starts_with(&self.root) {
            return Err(VfsError::PathEscape(resolved));
        }

        let canonical = match fs::canonicalize(&resolved) {
            Ok(c) => c,
            Err(_) => {
                if let Some(parent) = resolved.parent() {
                    if parent.exists() {
                        match fs::canonicalize(parent) {
                            Ok(canon_parent) => {
                                let fname = resolved.file_name().unwrap_or_default();
                                canon_parent.join(fname)
                            }
                            Err(_) => resolved,
                        }
                    } else {
                        resolved
                    }
                } else {
                    resolved
                }
            }
        };

        if !canonical.starts_with(&self.root) {
            return Err(VfsError::PathEscape(canonical));
        }

        Ok(canonical)
    }

    pub fn to_vpath(&self, abs_path: &Path) -> String {
        match abs_path.strip_prefix(&self.root) {
            Ok(rel) => {
                let s = rel.to_string_lossy();
                if s.is_empty() {
                    "/".to_string()
                } else {
                    format!("/{}", s)
                }
            }
            Err(_) => abs_path.to_string_lossy().to_string(),
        }
    }

    pub fn exists(&self, path: &str, cwd: &str) -> bool {
        self.resolve(path, cwd)
            .map(|p| p.exists())
            .unwrap_or(false)
    }

    pub fn is_dir(&self, path: &str, cwd: &str) -> bool {
        self.resolve(path, cwd)
            .map(|p| p.is_dir())
            .unwrap_or(false)
    }

    pub fn is_file(&self, path: &str, cwd: &str) -> bool {
        self.resolve(path, cwd)
            .map(|p| p.is_file())
            .unwrap_or(false)
    }

    pub fn create_dir(&self, path: &str, cwd: &str) -> Result<()> {
        let target = self.resolve(path, cwd)?;
        if target.exists() {
            return Err(VfsError::AlreadyExists(
                self.to_vpath(&target),
            ));
        }
        fs::create_dir(&target)?;
        Ok(())
    }

    pub fn create_dir_all(&self, path: &str, cwd: &str) -> Result<()> {
        let target = self.resolve(path, cwd)?;
        fs::create_dir_all(&target)?;
        Ok(())
    }

    pub fn read_to_string(&self, path: &str, cwd: &str) -> Result<String> {
        let target = self.resolve(path, cwd)?;
        if !target.exists() {
            return Err(VfsError::NotFound(self.to_vpath(&target)));
        }
        Ok(fs::read_to_string(&target)?)
    }

    pub fn read(&self, path: &str, cwd: &str) -> Result<Vec<u8>> {
        let target = self.resolve(path, cwd)?;
        if !target.exists() {
            return Err(VfsError::NotFound(self.to_vpath(&target)));
        }
        Ok(fs::read(&target)?)
    }

    pub fn write(&self, path: &str, cwd: &str, contents: &str) -> Result<()> {
        let target = self.resolve(path, cwd)?;
        if let Some(parent) = target.parent() {
            if !parent.exists() {
                fs::create_dir_all(parent)?;
            }
        }
        fs::write(&target, contents)?;
        Ok(())
    }

    pub fn write_bytes(&self, path: &str, cwd: &str, contents: &[u8]) -> Result<()> {
        let target = self.resolve(path, cwd)?;
        if let Some(parent) = target.parent() {
            if !parent.exists() {
                fs::create_dir_all(parent)?;
            }
        }
        fs::write(&target, contents)?;
        Ok(())
    }

    pub fn remove_file(&self, path: &str, cwd: &str) -> Result<()> {
        let target = self.resolve(path, cwd)?;
        if !target.exists() {
            return Err(VfsError::NotFound(self.to_vpath(&target)));
        }
        if target.is_dir() {
            return Err(VfsError::NotADirectory(self.to_vpath(&target)));
        }
        fs::remove_file(&target)?;
        Ok(())
    }

    pub fn remove_dir(&self, path: &str, cwd: &str) -> Result<()> {
        let target = self.resolve(path, cwd)?;
        if !target.exists() {
            return Err(VfsError::NotFound(self.to_vpath(&target)));
        }
        if !target.is_dir() {
            return Err(VfsError::NotADirectory(self.to_vpath(&target)));
        }
        fs::remove_dir(&target)?;
        Ok(())
    }

    pub fn remove_dir_all(&self, path: &str, cwd: &str) -> Result<()> {
        let target = self.resolve(path, cwd)?;
        if !target.exists() {
            return Err(VfsError::NotFound(self.to_vpath(&target)));
        }
        fs::remove_dir_all(&target)?;
        Ok(())
    }

    pub fn list_dir(&self, path: &str, cwd: &str) -> Result<Vec<DirEntry>> {
        let target = self.resolve(path, cwd)?;
        if !target.exists() {
            return Err(VfsError::NotFound(self.to_vpath(&target)));
        }
        if !target.is_dir() {
            return Err(VfsError::NotADirectory(self.to_vpath(&target)));
        }

        let mut entries = Vec::new();
        for entry in fs::read_dir(&target)? {
            let entry = entry?;
            let metadata = entry.metadata()?;
            entries.push(DirEntry {
                name: entry.file_name().to_string_lossy().to_string(),
                is_dir: metadata.is_dir(),
                size: metadata.len(),
                modified: metadata.modified().ok(),
            });
        }
        entries.sort_by(|a, b| {
            a.is_dir
                .cmp(&b.is_dir)
                .reverse()
                .then(a.name.to_lowercase().cmp(&b.name.to_lowercase()))
        });
        Ok(entries)
    }

    pub fn copy(&self, from: &str, to: &str, cwd: &str) -> Result<()> {
        let src = self.resolve(from, cwd)?;
        if !src.exists() {
            return Err(VfsError::NotFound(self.to_vpath(&src)));
        }
        let dst = self.resolve(to, cwd)?;

        if src.is_dir() {
            copy_dir_recursive(&src, &dst)?;
        } else {
            if let Some(parent) = dst.parent() {
                if !parent.exists() {
                    fs::create_dir_all(parent)?;
                }
            }
            fs::copy(&src, &dst)?;
        }
        Ok(())
    }

    pub fn rename(&self, from: &str, to: &str, cwd: &str) -> Result<()> {
        let src = self.resolve(from, cwd)?;
        if !src.exists() {
            return Err(VfsError::NotFound(self.to_vpath(&src)));
        }
        let dst = self.resolve(to, cwd)?;
        if let Some(parent) = dst.parent() {
            if !parent.exists() {
                fs::create_dir_all(parent)?;
            }
        }
        fs::rename(&src, &dst)?;
        Ok(())
    }

    pub fn metadata_len(&self, path: &str, cwd: &str) -> Result<u64> {
        let target = self.resolve(path, cwd)?;
        Ok(target.metadata()?.len())
    }
}

fn copy_dir_recursive(src: &Path, dst: &Path) -> Result<()> {
    fs::create_dir_all(dst)?;
    for entry in fs::read_dir(src)? {
        let entry = entry?;
        let file_type = entry.file_type()?;
        let dst_path = dst.join(entry.file_name());
        if file_type.is_dir() {
            copy_dir_recursive(&entry.path(), &dst_path)?;
        } else {
            fs::copy(entry.path(), &dst_path)?;
        }
    }
    Ok(())
}

fn normalize_path(path: &Path) -> PathBuf {
    let mut components = Vec::new();

    for component in path.components() {
        match component {
            std::path::Component::ParentDir => {
                components.pop();
            }
            std::path::Component::CurDir => {}
            c => components.push(c),
        }
    }

    if components.is_empty() {
        PathBuf::from("/")
    } else {
        components.iter().collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::sync::atomic::{AtomicUsize, Ordering};

    static TEST_COUNTER: AtomicUsize = AtomicUsize::new(0);

    fn setup_vfs() -> Vfs {
        let n = TEST_COUNTER.fetch_add(1, Ordering::SeqCst);
        let dir = std::env::temp_dir()
            .join(format!("fastshell_vfs_test_{}_{}", std::process::id(), n));
        let _ = fs::remove_dir_all(&dir);
        Vfs::new(dir).unwrap()
    }

    #[test]
    fn test_new_creates_root() {
        let dir = std::env::temp_dir().join(format!("fastshell_vfs_new_{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        let vfs = Vfs::new(dir.clone()).unwrap();
        assert!(vfs.root().exists());
        assert!(vfs.root().is_dir());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_resolve_absolute_path() {
        let vfs = setup_vfs();
        let resolved = vfs.resolve("/home/user", "").unwrap();
        assert!(resolved.starts_with(vfs.root()));
        assert!(resolved.ends_with("home/user"));
    }

    #[test]
    fn test_resolve_relative_path() {
        let vfs = setup_vfs();
        let resolved = vfs.resolve("foo/bar", "/home").unwrap();
        assert!(resolved.ends_with("home/foo/bar"));
    }

    #[test]
    fn test_resolve_prevents_escape() {
        let vfs = setup_vfs();
        let result = vfs.resolve("../../../etc/passwd", "/");
        assert!(result.is_err());
        match result {
            Err(VfsError::PathEscape(_)) => {}
            _ => panic!("Expected PathEscape error"),
        }
    }

    #[test]
    fn test_create_and_list_dir() {
        let vfs = setup_vfs();
        vfs.create_dir("/testdir", "").unwrap();
        assert!(vfs.is_dir("/testdir", ""));
        vfs.write("/testdir/file.txt", "", "hello").unwrap();
        let entries = vfs.list_dir("/testdir", "").unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].name, "file.txt");
    }

    #[test]
    fn test_read_write() {
        let vfs = setup_vfs();
        vfs.write("/hello.txt", "", "Hello, world!").unwrap();
        let content = vfs.read_to_string("/hello.txt", "").unwrap();
        assert_eq!(content, "Hello, world!");
    }

    #[test]
    fn test_remove_file() {
        let vfs = setup_vfs();
        vfs.write("/temp.txt", "", "data").unwrap();
        assert!(vfs.exists("/temp.txt", ""));
        vfs.remove_file("/temp.txt", "").unwrap();
        assert!(!vfs.exists("/temp.txt", ""));
    }

    #[test]
    fn test_copy_file() {
        let vfs = setup_vfs();
        vfs.write("/src.txt", "", "copy me").unwrap();
        vfs.copy("/src.txt", "/dst.txt", "").unwrap();
        assert_eq!(vfs.read_to_string("/dst.txt", "").unwrap(), "copy me");
    }

    #[test]
    fn test_rename() {
        let vfs = setup_vfs();
        vfs.write("/old.txt", "", "rename me").unwrap();
        vfs.rename("/old.txt", "/new.txt", "").unwrap();
        assert!(!vfs.exists("/old.txt", ""));
        assert_eq!(vfs.read_to_string("/new.txt", "").unwrap(), "rename me");
    }

    #[test]
    fn test_to_vpath() {
        let vfs = setup_vfs();
        let abs = vfs.resolve("/foo/bar", "").unwrap();
        assert_eq!(vfs.to_vpath(&abs), "/foo/bar");
        assert_eq!(vfs.to_vpath(vfs.root()), "/");
    }
}
