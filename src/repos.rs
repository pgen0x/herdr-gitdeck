//! Find the repos to show: the enclosing repo when started inside one,
//! otherwise every repo below the start folder.

use crate::git;
use std::path::{Path, PathBuf};

/// How deep to look below a non-repo folder (~/projects/org/repo is depth 2).
const MAX_DEPTH: usize = 3;
const SKIP_DIRS: &[&str] = &["node_modules", "target", "vendor", "dist", "build", "venv"];

pub fn discover(start: &Path) -> Vec<PathBuf> {
    if let Some(top) = git::toplevel(start) {
        return vec![PathBuf::from(top)];
    }
    let mut found = Vec::new();
    walk(start, 0, &mut found);
    found.sort();
    found
}

fn walk(dir: &Path, depth: usize, found: &mut Vec<PathBuf>) {
    if dir.join(".git").exists() {
        found.push(dir.to_path_buf());
        return; // don't descend into a repo: nested repos are its business
    }
    if depth >= MAX_DEPTH {
        return;
    }
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let name = entry.file_name();
        let name = name.to_string_lossy();
        let is_dir = entry.file_type().map(|t| t.is_dir()).unwrap_or(false);
        if is_dir && !name.starts_with('.') && !SKIP_DIRS.contains(&name.as_ref()) {
            walk(&entry.path(), depth + 1, found);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_nested_repos_and_skips_ignored_dirs() {
        let root = std::env::temp_dir().join(format!("gitdeck-test-{}", std::process::id()));
        let mk = |p: &str| std::fs::create_dir_all(root.join(p)).unwrap();
        mk("a/.git");
        mk("org/b/.git");
        mk("a/inner/.git"); // inside repo a: not listed
        mk("node_modules/c/.git");
        mk(".hidden/d/.git");
        let got: Vec<String> = discover(&root)
            .iter()
            .map(|p| p.strip_prefix(&root).unwrap().display().to_string())
            .collect();
        std::fs::remove_dir_all(&root).unwrap();
        assert_eq!(got, vec!["a", "org/b"]);
    }
}
