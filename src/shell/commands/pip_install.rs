// Copyright (c) 2025 xiefujin <490021684@qq.com>
// Licensed under Apache-2.0, see LICENSE file for full license terms.

use crate::shell::{CommandOutput, Shell};
use std::io::Cursor;

const PIP_INSTALL_HELP: &str = "\
Usage: pip-install [OPTIONS] [PACKAGE]...
       pip install [PACKAGE]...
       pip list

Install pure-Python packages from PyPI into site-packages/.

Only py3-none-any.whl (pure Python, no C extensions) are accepted.
Packages with .so/.pyd files are rejected with an error.

Options:
  -l, --list   list installed packages
  -h, --help   display this help and exit
";

const PYPI_JSON_URL: &str = "https://pypi.org/pypi/{}/json";

impl Shell {
    pub fn cmd_pip_install(&self, args: &[&str]) -> CommandOutput {
        let mut packages = Vec::new();
        let mut list_mode = false;

        for arg in args {
            match *arg {
                "-h" | "--help" => return CommandOutput::success(PIP_INSTALL_HELP.to_string()),
                "-l" | "--list" | "list" => list_mode = true,
                a if !a.starts_with('-') => packages.push(a.to_string()),
                _ => {}
            }
        }

        if list_mode {
            return self.cmd_pip_list();
        }

        if packages.is_empty() {
            return CommandOutput::error(
                "pip-install: missing package name(s)\n".to_string(),
                1,
            );
        }

        let cache_dir = std::env::var("FASTSHELL_WHEEL_CACHE")
            .ok()
            .map(std::path::PathBuf::from)
            .unwrap_or_else(|| std::path::PathBuf::from(".wheel_cache"));

        let site_dir = "site-packages";

        let agent = build_agent();

        let mut output = String::new();
        let mut failures = 0;

        for pkg in &packages {
            match install_package(&agent, pkg, &cache_dir, site_dir, &self.vfs, &self.cwd) {
                Ok(msg) => {
                    output.push_str(&format!("installed {}\n", pkg));
                    if !msg.is_empty() {
                        output.push_str(&format!("  {}\n", msg));
                    }
                }
                Err(e) => {
                    output.push_str(&format!("{}: error: {}\n", pkg, e));
                    failures += 1;
                }
            }
        }

        if failures > 0 {
            CommandOutput::error(output, 1)
        } else {
            CommandOutput::success(output)
        }
    }

    fn cmd_pip_list(&self) -> CommandOutput {
        let site_dir = "site-packages";
        let entries = match self.vfs.list_dir(site_dir, &self.cwd) {
            Ok(e) => e,
            Err(_) => return CommandOutput::success("(no packages installed)\n".to_string()),
        };

        let mut installed: std::collections::HashSet<String> = std::collections::HashSet::new();

        for entry in &entries {
            // Skip Python bytecode cache and other metadata dirs.
            if entry.name == "__pycache__" || entry.name == "site-packages" {
                continue;
            }
            if entry.is_dir {
                if entry.name.ends_with(".dist-info") {
                    // dist-info dir name is versioned (e.g. six-1.17.0.dist-info).
                    // Read the true package name from METADATA; fall back to
                    // stripping the version suffix from the dir name.
                    let metadata_path = format!("{}/{}/METADATA", site_dir, entry.name);
                    let pkg_name = self
                        .vfs
                        .read_to_string(&metadata_path, &self.cwd)
                        .ok()
                        .and_then(|c| extract_name_from_metadata(&c))
                        .unwrap_or_else(|| strip_dist_info_version(&entry.name));
                    if !pkg_name.is_empty() {
                        installed.insert(pkg_name);
                    }
                } else {
                    installed.insert(entry.name.clone());
                }
            } else if entry.name.ends_with(".py") {
                // Single-file module — recognized by a matching versioned
                // dist-info dir (e.g. six.py ↔ six-1.17.0.dist-info).
                let module_name = entry.name.trim_end_matches(".py");
                let has_dist_info = entries.iter().any(|e| {
                    e.is_dir
                        && e.name.ends_with(".dist-info")
                        && e.name.starts_with(&format!("{}-", module_name))
                });
                if has_dist_info {
                    installed.insert(module_name.to_string());
                }
            }
        }

        if installed.is_empty() {
            return CommandOutput::success("(no packages installed)\n".to_string());
        }

        let mut names: Vec<&String> = installed.iter().collect();
        names.sort();

        let mut output = String::new();
        for name in names {
            output.push_str(&format!("{}\n", name));
        }
        CommandOutput::success(output)
    }
}

fn build_agent() -> ureq::Agent {
    ureq::AgentBuilder::new()
        .timeout_connect(std::time::Duration::from_secs(15))
        .timeout(std::time::Duration::from_secs(60))
        .tls_config(build_tls())
        .build()
}

fn build_tls() -> std::sync::Arc<rustls::ClientConfig> {
    use rustls::crypto::CryptoProvider;
    let provider = CryptoProvider {
        cipher_suites: rustls::crypto::aws_lc_rs::default_provider().cipher_suites.to_vec(),
        ..rustls::crypto::aws_lc_rs::default_provider()
    };
    let root_store =
        rustls::RootCertStore::from_iter(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
    let config = rustls::ClientConfig::builder_with_provider(provider.into())
        .with_safe_default_protocol_versions()
        .unwrap()
        .with_root_certificates(root_store)
        .with_no_client_auth();
    std::sync::Arc::new(config)
}

fn install_package(
    agent: &ureq::Agent,
    pkg: &str,
    cache_dir: &std::path::Path,
    site_dir: &str,
    vfs: &crate::vfs::Vfs,
    cwd: &str,
) -> Result<String, String> {
    // 1. Parse package name and optional version
    let (pkg_name, version) = parse_pkg_spec(pkg);

    // 2. Query PyPI
    let info = fetch_pypi_info(agent, pkg_name)?;

    // 3. Find the best matching wheel for the requested version (or latest)
    let wheel_file = find_best_wheel(&info, version).ok_or_else(|| {
        if let Some(v) = version {
            format!("no py3-none-any.whl found for {} version {}", pkg_name, v)
        } else {
            format!(
                "no py3-none-any.whl found for {} (may require C extensions or single-file wheel)",
                pkg_name
            )
        }
    })?;

    let wheel_url = &wheel_file.url;
    let wheel_name = wheel_url.rsplit('/').next().unwrap_or(pkg_name);

    // 4. Check cache
    let cached = cache_dir.join(wheel_name);
    let wheel_data = if cached.exists() {
        let mut data = Vec::new();
        std::io::Read::read_to_end(
            &mut std::fs::File::open(&cached).map_err(|e| e.to_string())?,
            &mut data,
        )
        .map_err(|e| e.to_string())?;
        data
    } else {
        // 5. Download
        let resp = agent
            .get(wheel_url)
            .call()
            .map_err(|e| format!("download failed: {}", e))?;
        let mut data = Vec::new();
        std::io::Read::read_to_end(&mut resp.into_reader(), &mut data)
            .map_err(|e| format!("read failed: {}", e))?;

        // Save to cache
        if let Some(parent) = cached.parent() {
            let _ = std::fs::create_dir_all(parent);
            let _ = std::fs::write(&cached, &data);
        }
        data
    };

    // 6. Validate — scan for C extensions
    validate_pure_python_wheel(&wheel_data, pkg_name)?;

    // 7. Extract
    extract_wheel(&wheel_data, site_dir, vfs, cwd)?;

    Ok(String::new())
}

struct WheelFile {
    filename: String,
    url: String,
}

fn parse_pkg_spec(pkg: &str) -> (&str, Option<&str>) {
    if let Some(pos) = pkg.find("==") {
        (&pkg[..pos], Some(&pkg[pos + 2..]))
    } else {
        (pkg, None)
    }
}

/// Extract the `Name:` field from a wheel METADATA file.
fn extract_name_from_metadata(content: &str) -> Option<String> {
    for line in content.lines() {
        if let Some(v) = line.strip_prefix("Name:") {
            let name = v.trim();
            if !name.is_empty() {
                return Some(name.to_string());
            }
        }
    }
    None
}

/// Fallback package-name extraction from a versioned dist-info dir name
/// (e.g. `six-1.17.0.dist-info` → `six`,
/// `charset-normalizer-3.5.0.dist-info` → `charset-normalizer`). Scans from
/// the right for a `-` followed by a digit (the version), so names containing
/// hyphens survive.
fn strip_dist_info_version(dist_info: &str) -> String {
    let base = dist_info.trim_end_matches(".dist-info");
    for (i, ch) in base.char_indices().rev() {
        if ch == '-' {
            let rest = &base[i + 1..];
            if rest.chars().next().map_or(false, |c| c.is_ascii_digit()) {
                return base[..i].to_string();
            }
        }
    }
    base.to_string()
}

fn find_best_wheel<'a>(
    info: &'a [WheelFile],
    version: Option<&str>,
) -> Option<&'a WheelFile> {
    let candidates: Vec<&WheelFile> = info
        .iter()
        .filter(|f| {
            let name = &f.filename;
            // Accept pure-Python wheels (any py* tag, "none" ABI, "any" platform)
            // e.g. *-py3-none-any.whl, *-py2.py3-none-any.whl
            name.contains("-none-any.whl")
        })
        .collect();

    if let Some(ver) = version {
        candidates
            .into_iter()
            .rev()
            .find(|f| f.filename.contains(ver))
    } else {
        candidates.into_iter().last()
    }
}

fn fetch_pypi_info(agent: &ureq::Agent, pkg: &str) -> Result<Vec<WheelFile>, String> {
    let url = PYPI_JSON_URL.replace("{}", pkg);
    let resp = agent
        .get(&url)
        .call()
        .map_err(|e| format!("PyPI query failed: {}", e))?;
    let mut body = Vec::new();
    std::io::Read::read_to_end(&mut resp.into_reader(), &mut body)
        .map_err(|e| format!("read failed: {}", e))?;

    let json: serde_json::Value =
        serde_json::from_slice(&body).map_err(|e| format!("PyPI JSON parse error: {}", e))?;

    let mut result = Vec::new();
    let releases = json["releases"].as_object().ok_or("no releases field")?;

    // Process versions in order
    let mut versions: Vec<&String> = releases.keys().collect();
    versions.sort_by_key(|v| parse_version(v));

    for ver in &versions {
        if let Some(files) = releases[*ver].as_array() {
            for file in files {
                let filename = file["filename"].as_str().unwrap_or("");
                let url = file["url"].as_str().unwrap_or("");
                if !filename.is_empty() && !url.is_empty() {
                    result.push(WheelFile {
                        filename: filename.to_string(),
                        url: url.to_string(),
                    });
                }
            }
        }
    }

    if result.is_empty() {
        return Err(format!("no releases found for {}", pkg));
    }

    Ok(result)
}

fn parse_version(v: &str) -> Vec<i64> {
    v.split(|c: char| !c.is_ascii_digit())
        .filter(|s| !s.is_empty())
        .filter_map(|s| s.parse::<i64>().ok())
        .collect()
}

fn validate_pure_python_wheel(data: &[u8], pkg: &str) -> Result<(), String> {
    let cursor = Cursor::new(data);
    let mut archive =
        zip::ZipArchive::new(cursor).map_err(|e| format!("bad wheel file: {}", e))?;

    for i in 0..archive.len() {
        let entry = archive.by_index(i).map_err(|e| format!("read wheel: {}", e))?;
        let name = entry.name();
        if name.ends_with(".so") || name.ends_with(".pyd") {
            return Err(format!(
                "{} contains compiled C extension ({}) — cannot install on RustPython. Only pure-Python packages (py3-none-any.whl) are supported.",
                pkg, name
            ));
        }
    }

    Ok(())
}

fn extract_wheel(
    data: &[u8],
    site_dir: &str,
    vfs: &crate::vfs::Vfs,
    cwd: &str,
) -> Result<(), String> {
    let cursor = Cursor::new(data);
    let mut archive =
        zip::ZipArchive::new(cursor).map_err(|e| format!("bad wheel file: {}", e))?;

    // Classify: single-file module (e.g. six.py) or directory package
    let (is_single_file, pkg_root) = classify_package(&mut archive)?;

    if is_single_file {
        // Single-file module: extract directly into site-packages/
        // e.g. six.py → site-packages/six.py
        extract_single_file_wheel(&mut archive, &pkg_root, site_dir, vfs, cwd)?;
    } else {
        // Directory package: extract into site-packages/<module-dir>/.
        // Use the wheel's actual module directory name (pkg_root), NOT the
        // distribution name (pkg_name). They differ for many packages
        // (charset-normalizer → charset_normalizer, python-dateutil → dateutil,
        // pyyaml → yaml, beautifulsoup4 → bs4), and Python imports by the
        // module name — a dir named with a hyphen would be unimportable.
        let dest_dir = format!("{}/{}", site_dir, pkg_root);
        extract_directory_wheel(&mut archive, &pkg_root, &dest_dir, site_dir, vfs, cwd)?;
    }

    Ok(())
}

fn classify_package<R: std::io::Read + std::io::Seek>(
    archive: &mut zip::ZipArchive<R>,
) -> Result<(bool, String), String> {
    let mut pkg_root = String::new();
    let mut has_dir_entry = false;

    for i in 0..archive.len() {
        let entry = archive.by_index(i).map_err(|e| format!("read wheel: {}", e))?;
        let name = entry.name().to_string();

        if name.contains(".dist-info/") || name.contains(".data/") {
            continue;
        }

        let top = name.split('/').next().unwrap_or("");
        if top.is_empty()
            || top.ends_with(".dist-info")
            || top.ends_with(".data")
        {
            continue;
        }

        if pkg_root.is_empty() {
            pkg_root = top.to_string();
        }

        if name.contains('/') && name != format!("{}/", pkg_root) {
            has_dir_entry = true;
        }
    }

    if pkg_root.is_empty() {
        return Err("wheel contains no installable files".to_string());
    }

    // Single-file: top is a .py file and no subdirectory entries exist
    let is_single_file = !has_dir_entry && pkg_root.ends_with(".py");

    Ok((is_single_file, pkg_root))
}

fn extract_single_file_wheel<R: std::io::Read + std::io::Seek>(
    archive: &mut zip::ZipArchive<R>,
    pkg_root: &str,
    site_dir: &str,
    vfs: &crate::vfs::Vfs,
    cwd: &str,
) -> Result<(), String> {
    // Ensure site-packages directory exists
    let _ = vfs.create_dir_all(site_dir, cwd);

    for i in 0..archive.len() {
        let mut entry = archive.by_index(i).map_err(|e| format!("read wheel: {}", e))?;
        let name = entry.name().to_string();

        if entry.is_dir() {
            continue;
        }

        if name.contains(".dist-info/") {
            // Keep the wheel's original dist-info dir name (includes the
            // version, e.g. `six-1.17.0.dist-info`) instead of inventing
            // `six.dist-info`. Mirrors what real pip does.
            let out_path = format!("{}/{}", site_dir, name);
            let mut content = Vec::new();
            std::io::Read::read_to_end(&mut entry, &mut content)
                .map_err(|e| format!("read wheel entry {}: {}", name, e))?;
            if let Some(parent) = std::path::Path::new(&out_path).parent() {
                let _ = vfs.create_dir_all(&parent.to_string_lossy(), cwd);
            }
            vfs.write_bytes(&out_path, cwd, &content)
                .map_err(|e| format!("write {}: {}", out_path, e))?;
            continue;
        }

        if name.contains(".data/") {
            continue;
        }

        // The actual module file: extract as site-packages/<filename>
        let out_path = format!("{}/{}", site_dir, pkg_root);
        let mut content = Vec::new();
        std::io::Read::read_to_end(&mut entry, &mut content)
            .map_err(|e| format!("read wheel entry {}: {}", name, e))?;
        vfs.write_bytes(&out_path, cwd, &content)
            .map_err(|e| format!("write {}: {}", out_path, e))?;
    }

    Ok(())
}

fn extract_directory_wheel<R: std::io::Read + std::io::Seek>(
    archive: &mut zip::ZipArchive<R>,
    pkg_dir: &str,
    dest_dir: &str,
    site_dir: &str,
    vfs: &crate::vfs::Vfs,
    cwd: &str,
) -> Result<(), String> {
    let pkg_prefix = format!("{}/", pkg_dir);

    for i in 0..archive.len() {
        let mut entry = archive.by_index(i).map_err(|e| format!("read wheel: {}", e))?;
        let name = entry.name().to_string();

        if name.contains(".data/") {
            continue;
        }

        let is_dist_info = name.contains(".dist-info/");

        let relative = if name.starts_with(&pkg_prefix) && pkg_prefix.len() > 0 {
            &name[pkg_prefix.len()..]
        } else if name == pkg_dir || name == format!("{}/", pkg_dir) {
            continue;
        } else {
            &name
        };

        // dist-info lives at the top level of site-packages (keeps the
        // wheel's versioned name, e.g. `requests-2.31.0.dist-info`), while
        // package files go under `dest_dir` (site-packages/<module>).
        let out_path = if is_dist_info {
            format!("{}/{}", site_dir, name)
        } else {
            format!("{}/{}", dest_dir, relative)
        };
        if entry.is_dir() {
            let _ = vfs.create_dir(&out_path, cwd);
            continue;
        }

        let mut content = Vec::new();
        std::io::Read::read_to_end(&mut entry, &mut content)
            .map_err(|e| format!("read wheel entry {}: {}", name, e))?;

        if let Some(parent) = std::path::Path::new(&out_path).parent() {
            let parent_str = parent.to_string_lossy();
            if !parent_str.is_empty() {
                let _ = vfs.create_dir_all(&parent_str, cwd);
            }
        }

        vfs.write_bytes(&out_path, cwd, &content)
            .map_err(|e| format!("write {}: {}", out_path, e))?;
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_version() {
        assert!(parse_version("1.0") > parse_version("0.9"));
        assert!(parse_version("2.31.0") > parse_version("2.3.0"));
        assert!(parse_version("2.10") > parse_version("2.9"));
    }

    #[test]
    fn test_parse_pkg_spec_without_version() {
        let (name, ver) = parse_pkg_spec("six");
        assert_eq!(name, "six");
        assert_eq!(ver, None);
    }

    #[test]
    fn test_parse_pkg_spec_with_version() {
        let (name, ver) = parse_pkg_spec("requests==2.31.0");
        assert_eq!(name, "requests");
        assert_eq!(ver, Some("2.31.0"));
    }

    #[test]
    fn test_parse_pkg_spec_with_version_in_name() {
        let (name, ver) = parse_pkg_spec("python-dateutil==2.8.2");
        assert_eq!(name, "python-dateutil");
        assert_eq!(ver, Some("2.8.2"));
    }

    #[test]
    fn test_find_best_wheel_version_match() {
        let info = vec![
            WheelFile {
                filename: "six-1.16.0-py2.py3-none-any.whl".into(),
                url: "https://example.com/six-1.16.0-py2.py3-none-any.whl".into(),
            },
            WheelFile {
                filename: "six-1.17.0-py2.py3-none-any.whl".into(),
                url: "https://example.com/six-1.17.0-py2.py3-none-any.whl".into(),
            },
        ];
        let w = find_best_wheel(&info, Some("1.16.0")).unwrap();
        assert!(w.filename.contains("1.16.0"));

        let w = find_best_wheel(&info, None).unwrap();
        assert!(w.filename.contains("1.17.0"));
    }

    #[test]
    fn test_find_best_wheel_py2_py3_matched() {
        let info = vec![WheelFile {
            filename: "six-1.17.0-py2.py3-none-any.whl".into(),
            url: "https://example.com/six-1.17.0.whl".into(),
        }];
        let w = find_best_wheel(&info, None).unwrap();
        assert!(w.filename.contains("six"));
    }

    #[test]
    fn test_find_best_wheel_rejects_cp_specific() {
        let info = vec![WheelFile {
            filename: "numpy-1.26.0-cp312-cp312-manylinux.whl".into(),
            url: "https://example.com/numpy.whl".into(),
        }];
        assert!(find_best_wheel(&info, None).is_none());
    }

    #[test]
    fn test_classify_single_file_package() {
        // six-like: single six.py + dist-info
        let data = make_test_wheel(&[
            ("six.py", b"def foo(): pass\n"),
            ("six-1.17.0.dist-info/METADATA", b"Name: six\nVersion: 1.17.0\n"),
        ]);
        let cursor = Cursor::new(data);
        let mut archive = zip::ZipArchive::new(cursor).unwrap();
        let (is_single, root) = classify_package(&mut archive).unwrap();
        assert!(is_single, "six should be classified as single-file");
        assert_eq!(root, "six.py");
    }

    #[test]
    fn test_classify_directory_package() {
        // requests-like: directory with __init__.py
        let data = make_test_wheel(&[
            ("requests/__init__.py", b"from .api import get\n"),
            ("requests/api.py", b"def get(): pass\n"),
            ("requests-2.31.0.dist-info/METADATA", b"Name: requests\n"),
        ]);
        let cursor = Cursor::new(data);
        let mut archive = zip::ZipArchive::new(cursor).unwrap();
        let (is_single, root) = classify_package(&mut archive).unwrap();
        assert!(!is_single, "requests should be classified as directory");
        assert_eq!(root, "requests");
    }

    #[test]
    fn test_extract_single_file_wheel_vfs() {
        use crate::vfs::Vfs;
        let dir = std::env::temp_dir().join(format!(
            "fastshell_pip_extract_single_{}_{}",
            std::process::id(),
            uuid::Uuid::new_v4().simple()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        let vfs = Vfs::new(dir.clone()).unwrap();

        let data = make_test_wheel(&[
            ("six.py", b"def foo():\n    return 42\n"),
            ("six-1.17.0.dist-info/METADATA", b"Name: six\nVersion: 1.17.0\n"),
        ]);
        let cursor = Cursor::new(data);
        let mut archive = zip::ZipArchive::new(cursor).unwrap();
        let (is_single, root) = classify_package(&mut archive).unwrap();
        assert!(is_single);

        extract_single_file_wheel(&mut archive, &root, "site-packages", &vfs, "/").unwrap();

        // Verify the module file was extracted
        let entries = vfs.list_dir("site-packages", "/").unwrap();
        let has_six_py = entries.iter().any(|e| e.name == "six.py");
        assert!(has_six_py, "six.py should be extracted");

        // Verify dist-info was extracted with its versioned name
        let has_dist_info = entries
            .iter()
            .any(|e| e.name == "six-1.17.0.dist-info" && e.is_dir);
        assert!(has_dist_info, "six-1.17.0.dist-info should be extracted");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_extract_directory_wheel_vfs() {
        use crate::vfs::Vfs;
        let dir = std::env::temp_dir().join(format!(
            "fastshell_pip_extract_dir_{}_{}",
            std::process::id(),
            uuid::Uuid::new_v4().simple()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        let vfs = Vfs::new(dir.clone()).unwrap();

        let data = make_test_wheel(&[
            ("requests/__init__.py", b"from .api import get\n"),
            ("requests/api.py", b"def get(): pass\n"),
            ("requests/sub/__init__.py", b"x=1\n"),
            ("requests-2.31.0.dist-info/METADATA", b"Name: requests\n"),
        ]);
        let mut cursor = Cursor::new(data);
        let mut archive = zip::ZipArchive::new(cursor).unwrap();
        extract_directory_wheel(&mut archive, "requests", "site-packages/requests", "site-packages", &vfs, "/")
            .unwrap();

        let sub_entries = vfs.list_dir("site-packages/requests", "/").unwrap();
        assert!(sub_entries.iter().any(|e| e.name == "__init__.py"));
        assert!(sub_entries.iter().any(|e| e.name == "api.py"));

        // dist-info goes at site-packages top level (versioned name)
        let top_entries = vfs.list_dir("site-packages", "/").unwrap();
        assert!(
            top_entries
                .iter()
                .any(|e| e.name == "requests-2.31.0.dist-info" && e.is_dir),
            "requests-2.31.0.dist-info should be at site-packages top level"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_extract_wheel_single_file_integration() {
        use crate::vfs::Vfs;
        let dir = std::env::temp_dir().join(format!(
            "fastshell_pip_integrate_single_{}_{}",
            std::process::id(),
            uuid::Uuid::new_v4().simple()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        let vfs = Vfs::new(dir.clone()).unwrap();

        let mut cursor = Cursor::new(Vec::new());
        let mut writer = zip::ZipWriter::new(cursor);
        let opts =
            zip::write::FileOptions::default().compression_method(zip::CompressionMethod::Deflated);
        writer.start_file("six.py", opts).unwrap();
        std::io::Write::write_all(&mut writer, b"__version__ = '1.17.0'\n").unwrap();
        writer
            .start_file("six-1.17.0.dist-info/METADATA", opts)
            .unwrap();
        std::io::Write::write_all(&mut writer, b"Name: six\n").unwrap();
        let data = writer.finish().unwrap().into_inner();

        extract_wheel(&data, "site-packages", &vfs, "/").unwrap();

        let entries = vfs.list_dir("site-packages", "/").unwrap();
        assert!(
            entries.iter().any(|e| e.name == "six.py"),
            "six.py should be there"
        );
        assert!(
            entries
                .iter()
                .any(|e| e.name == "six-1.17.0.dist-info" && e.is_dir),
            "versioned dist-info should be there"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_extract_wheel_directory_integration() {
        use crate::vfs::Vfs;
        let dir = std::env::temp_dir().join(format!(
            "fastshell_pip_integrate_dir_{}_{}",
            std::process::id(),
            uuid::Uuid::new_v4().simple()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        let vfs = Vfs::new(dir.clone()).unwrap();

        let mut cursor = Cursor::new(Vec::new());
        let mut writer = zip::ZipWriter::new(cursor);
        let opts =
            zip::write::FileOptions::default().compression_method(zip::CompressionMethod::Deflated);
        writer.start_file("colorama/__init__.py", opts).unwrap();
        std::io::Write::write_all(&mut writer, b"from .ansi import AnsiCodes\n").unwrap();
        writer
            .start_file("colorama-0.4.6.dist-info/METADATA", opts)
            .unwrap();
        std::io::Write::write_all(&mut writer, b"Name: colorama\n").unwrap();
        let data = writer.finish().unwrap().into_inner();

        extract_wheel(&data, "site-packages", &vfs, "/").unwrap();

        let entries = vfs.list_dir("site-packages", "/").unwrap();
        let pkg = entries
            .iter()
            .find(|e| e.name == "colorama" && e.is_dir)
            .expect("colorama directory should exist");
        assert!(pkg.is_dir);

        let sub = vfs.list_dir("site-packages/colorama", "/").unwrap();
        assert!(sub.iter().any(|e| e.name == "__init__.py"));

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_extract_wheel_dist_name_differs_from_module() {
        use crate::vfs::Vfs;
        // Distribution name "charset-normalizer" (hyphen) has import module
        // "charset_normalizer" (underscore). Extraction must use the module
        // dir name, not the distribution name, or Python can't import it.
        let dir = std::env::temp_dir().join(format!(
            "fastshell_pip_dist_diff_{}_{}",
            std::process::id(),
            uuid::Uuid::new_v4().simple()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        let vfs = Vfs::new(dir.clone()).unwrap();

        let mut cursor = Cursor::new(Vec::new());
        let mut writer = zip::ZipWriter::new(cursor);
        let opts =
            zip::write::FileOptions::default().compression_method(zip::CompressionMethod::Deflated);
        writer.start_file("charset_normalizer/__init__.py", opts).unwrap();
        std::io::Write::write_all(&mut writer, b"__version__ = '3.5.0'\n").unwrap();
        writer.start_file("charset_normalizer/api.py", opts).unwrap();
        std::io::Write::write_all(&mut writer, b"def from_bytes(x): pass\n").unwrap();
        writer
            .start_file("charset_normalizer-3.5.0.dist-info/METADATA", opts)
            .unwrap();
        std::io::Write::write_all(&mut writer, b"Name: charset-normalizer\n").unwrap();
        let data = writer.finish().unwrap().into_inner();

        extract_wheel(&data, "site-packages", &vfs, "/").unwrap();

        let entries = vfs.list_dir("site-packages", "/").unwrap();
        // Must be charset_normalizer (underscore), NOT charset-normalizer (hyphen)
        assert!(
            entries.iter().any(|e| e.name == "charset_normalizer" && e.is_dir),
            "module dir should use underscore name, got: {:?}",
            entries.iter().map(|e| e.name.as_str()).collect::<Vec<_>>()
        );
        assert!(
            !entries.iter().any(|e| e.name == "charset-normalizer"),
            "hyphenated dist name should NOT be used as dir"
        );

        let sub = vfs.list_dir("site-packages/charset_normalizer", "/").unwrap();
        assert!(sub.iter().any(|e| e.name == "__init__.py"));

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_pip_list_with_installed_packages() {
        use crate::vfs::Vfs;
        let dir = std::env::temp_dir().join(format!(
            "fastshell_pip_list_{}_{}",
            std::process::id(),
            uuid::Uuid::new_v4().simple()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        let vfs = Vfs::new(dir.clone()).unwrap();
        let shell = Shell::new(vfs);

        // First, empty list
        let out = shell.cmd_pip_list();
        assert!(out.stdout.contains("(no packages installed)"));

        // Create a directory-based package
        shell
            .vfs
            .create_dir_all("site-packages/colorama", "/")
            .unwrap();
        shell
            .vfs
            .write_bytes(
                "site-packages/colorama/__init__.py",
                "/",
                b"x=1",
            )
            .unwrap();

        let out = shell.cmd_pip_list();
        assert!(out.stdout.contains("colorama"));
        assert!(!out.stdout.contains("(no packages installed)"));
        assert_eq!(out.exit_code, 0);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_pip_list_with_single_file_module() {
        use crate::vfs::Vfs;
        let dir = std::env::temp_dir().join(format!(
            "fastshell_pip_list_single_{}_{}",
            std::process::id(),
            uuid::Uuid::new_v4().simple()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        let vfs = Vfs::new(dir.clone()).unwrap();
        let shell = Shell::new(vfs);

        // Create a single-file module with its dist-info
        shell
            .vfs
            .create_dir_all("site-packages", "/")
            .unwrap();
        shell
            .vfs
            .write_bytes("site-packages/six.py", "/", b"x=1")
            .unwrap();
        shell
            .vfs
            .create_dir_all("site-packages/six-1.17.0.dist-info", "/")
            .unwrap();
        shell
            .vfs
            .write_bytes(
                "site-packages/six-1.17.0.dist-info/METADATA",
                "/",
                b"Name: six\nVersion: 1.17.0\n",
            )
            .unwrap();

        let out = shell.cmd_pip_list();
        assert!(
            out.stdout.contains("six"),
            "pip list should include single-file module: {}",
            out.stdout
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_pip_list_excludes_pycache() {
        use crate::vfs::Vfs;
        let dir = std::env::temp_dir().join(format!(
            "fastshell_pip_list_pycache_{}_{}",
            std::process::id(),
            uuid::Uuid::new_v4().simple()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        let vfs = Vfs::new(dir.clone()).unwrap();
        let shell = Shell::new(vfs);

        // A real package + a __pycache__ dir left over from an import.
        shell
            .vfs
            .create_dir_all("site-packages/colorama", "/")
            .unwrap();
        shell
            .vfs
            .create_dir_all("site-packages/__pycache__", "/")
            .unwrap();
        shell
            .vfs
            .write_bytes("site-packages/colorama/__init__.py", "/", b"x=1")
            .unwrap();

        let out = shell.cmd_pip_list();
        assert!(out.stdout.contains("colorama"), "colorama should be listed");
        assert!(
            !out.stdout.contains("__pycache__"),
            "__pycache__ should be excluded, got: {}",
            out.stdout
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_validate_no_c_extensions() {
        let data = make_test_wheel(&[("test/__init__.py", b"x=1"), ("test/module.py", b"y=2")]);
        assert!(validate_pure_python_wheel(&data, "test").is_ok());
    }

    #[test]
    fn test_validate_rejects_so() {
        let data = make_test_wheel(&[
            ("test/__init__.py", b"x=1"),
            ("test/_speedups.so", b"\x7fELF"),
        ]);
        assert!(validate_pure_python_wheel(&data, "test").is_err());
    }

    fn make_test_wheel(files: &[(&str, &[u8])]) -> Vec<u8> {
        let cursor = Cursor::new(Vec::new());
        let mut writer = zip::ZipWriter::new(cursor);
        let options =
            zip::write::FileOptions::default().compression_method(zip::CompressionMethod::Deflated);
        for (name, data) in files {
            writer.start_file(*name, options).unwrap();
            std::io::Write::write_all(&mut writer, data).unwrap();
        }
        writer.finish().unwrap().into_inner()
    }

    #[test]
    fn test_pip_install_help() {
        use crate::vfs::Vfs;
        let dir = std::env::temp_dir().join("fastshell_pip_test_help");
        let _ = std::fs::remove_dir_all(&dir);
        let vfs = Vfs::new(dir).unwrap();
        let shell = Shell::new(vfs);
        let out = shell.cmd_pip_install(&["-h"]);
        assert_eq!(out.exit_code, 0);
        assert!(out.stdout.contains("Usage: pip-install"));
    }

    #[test]
    fn test_pip_install_help_long() {
        use crate::vfs::Vfs;
        let dir = std::env::temp_dir().join("fastshell_pip_test_help2");
        let _ = std::fs::remove_dir_all(&dir);
        let vfs = Vfs::new(dir).unwrap();
        let shell = Shell::new(vfs);
        let out = shell.cmd_pip_install(&["--help"]);
        assert_eq!(out.exit_code, 0);
    }

    #[test]
    fn test_pip_install_no_args_error() {
        use crate::vfs::Vfs;
        let dir = std::env::temp_dir().join("fastshell_pip_test_noargs");
        let _ = std::fs::remove_dir_all(&dir);
        let vfs = Vfs::new(dir).unwrap();
        let shell = Shell::new(vfs);
        let out = shell.cmd_pip_install(&[]);
        assert_eq!(out.exit_code, 1);
    }
}
