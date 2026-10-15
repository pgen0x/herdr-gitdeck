//! Thin wrapper over the `git` CLI. Every call is `git -C <repo> ...`, so nothing
//! depends on the process cwd.

use anyhow::{bail, Context, Result};
use std::io::Write;
use std::path::Path;
use std::process::{Command, Output, Stdio};

/// Which side of the index a changed file lives on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Area {
    Staged,
    Unstaged,
    Untracked,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FileEntry {
    pub path: String,
    pub area: Area,
    /// Porcelain status letter: M, A, D, R, U, ?...
    pub code: char,
    /// Previous path of a rename.
    pub orig: Option<String>,
}

#[derive(Clone, Debug, Default)]
pub struct RepoStatus {
    pub branch: String,
    pub ahead: u32,
    pub behind: u32,
    /// Sorted: staged first, then unstaged, then untracked.
    pub files: Vec<FileEntry>,
}

impl RepoStatus {
    pub fn is_dirty(&self) -> bool {
        !self.files.is_empty()
    }

    pub fn count(&self, area: Area) -> usize {
        self.files.iter().filter(|f| f.area == area).count()
    }
}

fn git(repo: &Path) -> Command {
    let mut cmd = Command::new("git");
    cmd.arg("-C").arg(repo);
    // Plain output regardless of the user's git config, and `a/` `b/` prefixes that
    // `git apply` expects when a hunk patch goes back in.
    for opt in [
        "color.ui=never",
        "core.quotepath=off",
        "diff.noprefix=false",
        "diff.mnemonicPrefix=false",
        "diff.relative=false",
    ] {
        cmd.args(["-c", opt]);
    }
    // Read-only commands must not take index.lock: an agent may be committing right now.
    cmd.arg("--no-optional-locks");
    // Never read the TUI's terminal: a credential or pinentry prompt would freeze the pane.
    cmd.stdin(Stdio::null()).env("GIT_TERMINAL_PROMPT", "0");
    // Paths are file names, never globs.
    cmd.arg("--literal-pathspecs");
    cmd
}

fn check(out: Output, what: &str) -> Result<String> {
    if !out.status.success() {
        bail!("{what}: {}", String::from_utf8_lossy(&out.stderr).trim());
    }
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
}

/// Run git with `args` and return stdout, failing on a non-zero exit.
pub fn run(repo: &Path, args: &[&str]) -> Result<String> {
    let out = git(repo).args(args).output().context("failed to run git")?;
    check(out, &format!("git {}", args.join(" ")))
}

/// Run git feeding `input` on stdin (used for `git apply`).
fn run_stdin(repo: &Path, args: &[&str], input: &str) -> Result<String> {
    let mut child = git(repo)
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .context("failed to run git")?;
    // `take()` moves stdin out of the child so we own it and can close it.
    let mut stdin = child.stdin.take().context("git stdin unavailable")?;
    stdin.write_all(input.as_bytes())?;
    drop(stdin); // close it so git sees end of input
    check(
        child.wait_with_output()?,
        &format!("git {}", args.join(" ")),
    )
}

/// `Some(toplevel)` when `dir` is inside a git work tree.
pub fn toplevel(dir: &Path) -> Option<String> {
    run(dir, &["rev-parse", "--show-toplevel"])
        .ok()
        .map(|s| s.trim().to_string())
}

pub fn status(repo: &Path) -> Result<RepoStatus> {
    let raw = run(
        repo,
        &[
            "status",
            "--porcelain=v2",
            "--branch",
            "-z",
            "--untracked-files=normal",
        ],
    )?;
    Ok(parse_status(&raw))
}

/// Parse `git status --porcelain=v2 --branch -z`.
pub fn parse_status(raw: &str) -> RepoStatus {
    let mut st = RepoStatus::default();
    let mut entries = raw.split('\0');
    while let Some(entry) = entries.next() {
        if let Some(head) = entry.strip_prefix("# branch.head ") {
            st.branch = head.to_string();
        } else if let Some(ab) = entry.strip_prefix("# branch.ab ") {
            // "+1 -2"
            let mut it = ab.split_whitespace();
            st.ahead = it.next().and_then(|s| s[1..].parse().ok()).unwrap_or(0);
            st.behind = it.next().and_then(|s| s[1..].parse().ok()).unwrap_or(0);
        } else if let Some(path) = entry.strip_prefix("? ") {
            st.files.push(FileEntry {
                path: path.into(),
                area: Area::Untracked,
                code: '?',
                orig: None,
            });
        } else if entry.starts_with("1 ") || entry.starts_with("2 ") || entry.starts_with("u ") {
            // Field count before the path: ordinary 8, rename 9, unmerged 10.
            let fields = match &entry[..1] {
                "1" => 8,
                "2" => 9,
                _ => 10,
            };
            // A rename's original path follows as its own entry: always consume it,
            // even if this entry is malformed, so the parse stays in step.
            let orig = entry.starts_with('2').then(|| entries.next()).flatten();
            let parts: Vec<&str> = entry.splitn(fields + 1, ' ').collect();
            let (Some(xy), Some(path)) = (parts.get(1), parts.get(fields)) else {
                continue;
            };
            let mut xy = xy.chars();
            let (x, y) = (xy.next().unwrap_or('.'), xy.next().unwrap_or('.'));
            if entry.starts_with('u') {
                st.files.push(FileEntry {
                    path: path.to_string(),
                    area: Area::Unstaged,
                    code: 'U',
                    orig: None,
                });
                continue;
            }
            if x != '.' {
                st.files.push(FileEntry {
                    path: path.to_string(),
                    area: Area::Staged,
                    code: x,
                    orig: orig.map(String::from),
                });
            }
            if y != '.' {
                st.files.push(FileEntry {
                    path: path.to_string(),
                    area: Area::Unstaged,
                    code: y,
                    orig: None,
                });
            }
        }
    }
    st.files
        .sort_by(|a, b| (a.area as u8, &a.path).cmp(&(b.area as u8, &b.path)));
    st
}

/// Unified diff text for one file in one area.
pub fn diff(repo: &Path, file: &FileEntry) -> Result<String> {
    let path = file.path.as_str();
    // Raw content, not a textconv rendering: hunks must apply back to the real file.
    let base = ["diff", "--no-ext-diff", "--no-textconv"];
    match file.area {
        Area::Unstaged => run(repo, &[&base[..], &["--", path]].concat()),
        Area::Staged => {
            // Both sides of a rename, so git shows it as a rename, not an added file.
            let mut args = [&base[..], &["--cached", "--", path]].concat();
            args.extend(file.orig.as_deref());
            run(repo, &args)
        }
        // An untracked folder (`normal` mode collapses them) has no diff of its own.
        Area::Untracked if path.ends_with('/') => Ok(String::new()),
        Area::Untracked => {
            // `--no-index` exits 1 when files differ, which is always the case here.
            let out = git(repo)
                .args([
                    "diff",
                    "--no-index",
                    "--no-ext-diff",
                    "--no-textconv",
                    "--",
                    "/dev/null",
                    path,
                ])
                .output()?;
            if out.status.code() == Some(1) || out.status.success() {
                Ok(String::from_utf8_lossy(&out.stdout).into_owned())
            } else {
                check(out, "git diff --no-index")
            }
        }
    }
}

pub fn stage_file(repo: &Path, path: &str) -> Result<()> {
    run(repo, &["add", "--", path]).map(drop)
}

/// Unstage a file; for a rename, both paths, or the old path's deletion stays staged.
pub fn unstage_file(repo: &Path, file: &FileEntry) -> Result<()> {
    let mut args = vec!["restore", "--staged", "--", file.path.as_str()];
    args.extend(file.orig.as_deref());
    run(repo, &args).map(drop)
}

/// Throw away the working-tree changes of a file. Untracked files are deleted.
pub fn discard_file(repo: &Path, file: &FileEntry) -> Result<()> {
    match file.area {
        Area::Untracked => {
            std::fs::remove_file(repo.join(&file.path)).context("delete untracked file")
        }
        Area::Unstaged => run(repo, &["restore", "--", &file.path]).map(drop),
        Area::Staged => bail!("unstage the file before discarding it"),
    }
}

/// How a single-hunk patch is applied.
pub enum Apply {
    Stage,
    Unstage,
    Discard,
}

pub fn apply_patch(repo: &Path, patch: &str, how: Apply) -> Result<()> {
    let args: &[&str] = match how {
        Apply::Stage => &["apply", "--cached", "-"],
        Apply::Unstage => &["apply", "--cached", "-R", "-"],
        Apply::Discard => &["apply", "-R", "-"],
    };
    run_stdin(repo, args, patch).map(drop)
}

pub fn commit(repo: &Path, message: &str) -> Result<String> {
    let out = run(repo, &["commit", "-m", message])?;
    Ok(out.lines().next().unwrap_or_default().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    /// A throwaway repo with `files` committed; removed on drop.
    struct TempRepo(PathBuf);

    impl TempRepo {
        fn new(name: &str, files: &[(&str, &str)]) -> Self {
            let dir = std::env::temp_dir().join(format!("gitdeck-{name}-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&dir);
            std::fs::create_dir_all(&dir).unwrap();
            for (path, text) in files {
                std::fs::write(dir.join(path), text).unwrap();
            }
            let id = [
                "-c",
                "user.name=t",
                "-c",
                "user.email=t@t",
                "-c",
                "core.autocrlf=false",
            ];
            run(&dir, &["init", "-q"]).unwrap();
            run(&dir, &[&id[..], &["add", "."]].concat()).unwrap();
            let commit = ["commit", "-qm", "chore: init", "-m", "- test fixture"];
            run(&dir, &[&id[..], &commit[..]].concat()).unwrap();
            TempRepo(dir)
        }

        fn write(&self, path: &str, text: &str) {
            std::fs::write(self.0.join(path), text).unwrap();
        }

        fn entry(&self, path: &str, area: Area) -> FileEntry {
            FileEntry {
                path: path.into(),
                area,
                code: 'M',
                orig: None,
            }
        }
    }

    impl Drop for TempRepo {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn stages_and_unstages_a_single_hunk() {
        let original: String = (1..=30).map(|i| format!("line {i}\n")).collect();
        let repo = TempRepo::new("hunk", &[("f.txt", &original)]);
        // Two far-apart edits → two hunks.
        repo.write(
            "f.txt",
            &original
                .replace("line 2\n", "line TWO\n")
                .replace("line 28\n", "line 28!\n"),
        );

        let raw = diff(&repo.0, &repo.entry("f.txt", Area::Unstaged)).unwrap();
        let parsed = crate::diff::parse(&raw);
        assert_eq!(parsed.hunks.len(), 2);
        apply_patch(&repo.0, &parsed.hunk_patch(&raw, 1).unwrap(), Apply::Stage).unwrap();

        let staged = run(&repo.0, &["diff", "--cached"]).unwrap();
        assert!(staged.contains("+line 28!") && !staged.contains("TWO"));

        let raw = diff(&repo.0, &repo.entry("f.txt", Area::Staged)).unwrap();
        let patch = crate::diff::parse(&raw).hunk_patch(&raw, 0).unwrap();
        apply_patch(&repo.0, &patch, Apply::Unstage).unwrap();
        assert!(run(&repo.0, &["diff", "--cached"]).unwrap().is_empty());
    }

    #[test]
    fn stages_a_hunk_in_a_crlf_file() {
        let repo = TempRepo::new("crlf", &[("w.txt", "one\r\ntwo\r\nthree\r\n")]);
        repo.write("w.txt", "one\r\nTWO\r\nthree\r\n");
        let raw = diff(&repo.0, &repo.entry("w.txt", Area::Unstaged)).unwrap();
        let patch = crate::diff::parse(&raw).hunk_patch(&raw, 0).unwrap();
        apply_patch(&repo.0, &patch, Apply::Stage).unwrap();
        assert!(run(&repo.0, &["diff", "--cached"])
            .unwrap()
            .contains("+TWO"));
    }

    #[test]
    fn paths_are_never_globs() {
        let repo = TempRepo::new("glob", &[("x.rs", "a\n")]);
        repo.write("x.rs", "changed\n");
        // As a glob, `*.rs` would stage x.rs; as a literal path it matches nothing.
        assert!(stage_file(&repo.0, "*.rs").is_err());
        assert!(run(&repo.0, &["diff", "--cached"]).unwrap().is_empty());
    }

    #[test]
    fn unstaging_a_rename_restores_both_paths() {
        let repo = TempRepo::new("rename", &[("old.rs", "fn main() {}\n")]);
        run(&repo.0, &["mv", "old.rs", "new.rs"]).unwrap();
        let st = status(&repo.0).unwrap();
        let renamed = st.files.iter().find(|f| f.area == Area::Staged).unwrap();
        assert_eq!(
            (renamed.code, renamed.orig.as_deref()),
            ('R', Some("old.rs"))
        );
        unstage_file(&repo.0, renamed).unwrap();
        assert!(run(&repo.0, &["diff", "--cached"]).unwrap().is_empty());
    }

    #[test]
    fn parses_porcelain_v2() {
        let raw = "# branch.oid abc\0# branch.head main\0# branch.ab +2 -1\0\
                   1 M. N... 100644 100644 100644 aaa bbb src/a.rs\0\
                   1 .M N... 100644 100644 100644 aaa bbb src/b file.rs\0\
                   1 MM N... 100644 100644 100644 aaa bbb c.rs\0\
                   2 R. N... 100644 100644 100644 aaa bbb R100 new.rs\0old.rs\0\
                   u UU N... 100644 100644 100644 100644 a b c conflict.rs\0\
                   ? notes.txt\0";
        let st = parse_status(raw);
        assert_eq!(st.branch, "main");
        assert_eq!((st.ahead, st.behind), (2, 1));
        let got: Vec<(Area, &str, char)> = st
            .files
            .iter()
            .map(|f| (f.area, f.path.as_str(), f.code))
            .collect();
        assert_eq!(
            got,
            vec![
                (Area::Staged, "c.rs", 'M'),
                (Area::Staged, "new.rs", 'R'),
                (Area::Staged, "src/a.rs", 'M'),
                (Area::Unstaged, "c.rs", 'M'),
                (Area::Unstaged, "conflict.rs", 'U'),
                (Area::Unstaged, "src/b file.rs", 'M'),
                (Area::Untracked, "notes.txt", '?'),
            ]
        );
    }
}
