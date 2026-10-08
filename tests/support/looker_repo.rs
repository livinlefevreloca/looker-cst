//! Locates the looker repo checkout used by the tests and benchmarks, cloning it when needed.
//!
//! The repo is read from LOOKER_REPO, which defaults to target/looker. When that path does
//! not exist the repo is cloned there from LOOKER_REPO_URL (default mozilla/looker-hub over
//! HTTPS), so point LOOKER_REPO at an existing checkout to test a branch.
//! If the clone fails callers get no files, unless REQUIRE_LOOKER_REPO is set (as in CI),
//! where it panics.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::OnceLock;

const DEFAULT_REPO_URL: &str = "https://github.com/mozilla/looker-hub.git";

pub fn looker_files() -> Vec<PathBuf> {
    let Some(root) = looker_repo() else {
        return Vec::new();
    };
    let mut files = Vec::new();
    collect(root, &mut files);
    files.sort();
    assert!(!files.is_empty(), "no .lkml files under {}", root.display());
    files
}

/// The looker repo checkout, cloned on first use. Tests in this binary run in parallel, so
/// the lookup happens once and is shared.
fn looker_repo() -> Option<&'static Path> {
    static REPO: OnceLock<Option<PathBuf>> = OnceLock::new();
    REPO.get_or_init(|| {
        let root = std::env::var("LOOKER_REPO")
            .map(PathBuf::from)
            .unwrap_or_else(|_| Path::new(env!("CARGO_MANIFEST_DIR")).join("target/looker"));
        if root.is_dir() {
            return Some(root);
        }
        let url = std::env::var("LOOKER_REPO_URL").unwrap_or_else(|_| DEFAULT_REPO_URL.to_string());
        match clone(&url, &root) {
            Ok(()) => Some(root),
            Err(error) => {
                assert!(
                    std::env::var_os("REQUIRE_LOOKER_REPO").is_none(),
                    "could not clone {url} to {}: {error}",
                    root.display()
                );
                eprintln!("skipping: could not clone {url} to {}: {error}", root.display());
                None
            }
        }
    })
    .as_deref()
}

/// Shallow clones url to root, via a temporary directory so an interrupted clone is never
/// mistaken for a complete checkout.
fn clone(url: &str, root: &Path) -> Result<(), String> {
    let partial = root.with_extension(format!("partial-{}", std::process::id()));
    if let Some(parent) = root.parent() {
        fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    eprintln!("cloning {url} to {}", root.display());
    let output = Command::new("git")
        .args(["clone", "--quiet", "--depth", "1", url])
        .arg(&partial)
        .output()
        .map_err(|e| format!("could not run git: {e}"))?;
    if !output.status.success() {
        let _ = fs::remove_dir_all(&partial);
        return Err(String::from_utf8_lossy(&output.stderr).trim().to_string());
    }
    if fs::rename(&partial, root).is_err() {
        // Another test process finished its clone first; use that one.
        let _ = fs::remove_dir_all(&partial);
    }
    Ok(())
}

fn collect(dir: &Path, files: &mut Vec<PathBuf>) {
    for entry in fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            if path.file_name().is_some_and(|name| name != ".git") {
                collect(&path, files);
            }
        } else if path.extension().is_some_and(|ext| ext == "lkml") {
            files.push(path);
        }
    }
}
