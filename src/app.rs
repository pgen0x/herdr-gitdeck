//! Application state and key handling. Rendering lives in `ui.rs`.

use crate::diff::{self, FileDiff, Row};
use crate::git::{self, Apply, Area, FileEntry, RepoStatus};
use crate::{herdr, highlight, scan};
use anyhow::{bail, Context, Result};
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use std::path::PathBuf;
use std::sync::mpsc::{self, Receiver, Sender};

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Focus {
    Repos,
    Files,
    Diff,
}

/// A destructive operation waiting for `y`. The target is resolved when the prompt
/// opens, so a rescan that moves the selection meanwhile can't redirect it.
pub enum Pending {
    DiscardFile {
        repo: PathBuf,
        file: FileEntry,
    },
    DiscardHunk {
        repo: PathBuf,
        path: String,
        patch: String,
    },
}

pub enum Mode {
    Normal,
    Help,
    Commit(String),
    Comment(String),
    Confirm(Pending, String),
}

pub struct RepoView {
    pub path: PathBuf,
    pub name: String,
    pub status: Option<Result<RepoStatus, String>>,
}

impl RepoView {
    pub fn ok_status(&self) -> Option<&RepoStatus> {
        self.status.as_ref()?.as_ref().ok()
    }
    fn needs_attention(&self) -> bool {
        self.ok_status()
            .is_some_and(|s| s.is_dirty() || s.ahead > 0)
    }
}

pub struct Comment {
    pub repo: PathBuf,
    pub file: String,
    pub line: Option<u32>,
    /// `line` numbers the old file: the comment is on a deleted line.
    pub deleted: bool,
    pub snippet: String,
    pub body: String,
}

/// A highlighted diff and the raw text it was made from.
type Highlighted = (String, FileDiff);

pub struct App {
    pub root: PathBuf,
    pub repos: Vec<RepoView>,
    pub repo_sel: usize,
    pub hide_clean: bool,
    pub file_sel: usize,
    pub diff: Option<FileDiff>,
    diff_raw: String,
    pub rows: Vec<Row>,
    pub cursor: usize,
    pub scroll: usize,
    pub split: bool,
    pub focus: Focus,
    pub mode: Mode,
    pub message: String,
    pub comments: Vec<Comment>,
    /// Repo the open commit prompt will commit to.
    pub commit_repo: Option<PathBuf>,
    pub quit: bool,
    scanner: scan::Scanner,
    /// Highlighted diffs coming back from worker threads, keyed by their raw text.
    highlighted: (Sender<Highlighted>, Receiver<Highlighted>),
}

impl App {
    pub fn new(root: PathBuf, paths: Vec<PathBuf>, scanner: scan::Scanner) -> Self {
        let repos = paths
            .into_iter()
            .map(|path| {
                // Show the path relative to the start folder; the folder name when they match.
                let name = match path.strip_prefix(&root) {
                    Ok(rel) if !rel.as_os_str().is_empty() => rel.display().to_string(),
                    _ => path.file_name().map_or(path.display().to_string(), |n| {
                        n.to_string_lossy().into_owned()
                    }),
                };
                RepoView {
                    path,
                    name,
                    status: None,
                }
            })
            .collect();
        App {
            root,
            repos,
            repo_sel: 0,
            hide_clean: false,
            file_sel: 0,
            diff: None,
            diff_raw: String::new(),
            rows: Vec::new(),
            cursor: 0,
            scroll: 0,
            split: false,
            focus: Focus::Repos,
            mode: Mode::Normal,
            message: String::new(),
            comments: Vec::new(),
            commit_repo: None,
            quit: false,
            scanner,
            highlighted: mpsc::channel(),
        }
    }

    /// Apply finished background work (scans, highlighting). Returns how many results landed.
    pub fn tick(&mut self) -> usize {
        let mut landed = 0;
        while let Ok(scan) = self.scanner.results.try_recv() {
            self.apply_scan(scan);
            landed += 1;
        }
        while let Ok((raw, diff)) = self.highlighted.1.try_recv() {
            // Drop results for a diff that is no longer on screen.
            if raw == self.diff_raw {
                self.diff = Some(diff); // same lines as the plain one, so rows stay valid
                landed += 1;
            }
        }
        landed
    }

    // ---- derived views -------------------------------------------------------------

    /// Indices into `repos` in display order: dirty first, then by name.
    pub fn visible_repos(&self) -> Vec<usize> {
        let mut idx: Vec<usize> = (0..self.repos.len())
            .filter(|&i| !self.hide_clean || self.repos[i].needs_attention())
            .collect();
        idx.sort_by_key(|&i| (!self.repos[i].needs_attention(), self.repos[i].name.clone()));
        idx
    }

    pub fn current_repo(&self) -> Option<&RepoView> {
        self.visible_repos()
            .get(self.repo_sel)
            .map(|&i| &self.repos[i])
    }

    pub fn files(&self) -> &[FileEntry] {
        self.current_repo()
            .and_then(|r| r.ok_status())
            .map_or(&[], |s| &s.files)
    }

    pub fn current_file(&self) -> Option<&FileEntry> {
        self.files().get(self.file_sel)
    }

    /// The diff line under the cursor (new side preferred in split view).
    pub fn cursor_line(&self) -> Option<&diff::Line> {
        let diff = self.diff.as_ref()?;
        match *self.rows.get(self.cursor)? {
            Row::HunkHeader(_) => None,
            Row::Line { hunk, idx } => diff.hunks[hunk].lines.get(idx),
            Row::Pair { hunk, old, new } => diff.hunks[hunk].lines.get(new.or(old)?),
        }
    }

    pub fn has_comment(&self, file: &str, line: Option<u32>) -> bool {
        let repo = self.current_repo().map(|r| &r.path);
        self.comments
            .iter()
            .any(|c| Some(&c.repo) == repo && c.file == file && c.line == line)
    }

    // ---- state updates -------------------------------------------------------------

    /// Apply a finished background scan, keeping the selection on the same repo/file.
    pub fn apply_scan(&mut self, results: Vec<(PathBuf, Result<RepoStatus, String>)>) {
        let repo = self.current_repo().map(|r| r.path.clone());
        let file = self.current_file().map(|f| (f.path.clone(), f.area));
        for (path, status) in results {
            if let Some(r) = self.repos.iter_mut().find(|r| r.path == path) {
                r.status = Some(status);
            }
        }
        self.reselect(repo, file);
    }

    fn reselect(&mut self, repo: Option<PathBuf>, file: Option<(String, Area)>) {
        let vis = self.visible_repos();
        if let Some(pos) = vis
            .iter()
            .position(|&i| Some(&self.repos[i].path) == repo.as_ref())
        {
            self.repo_sel = pos;
        }
        self.repo_sel = self.repo_sel.min(vis.len().saturating_sub(1));
        let files = self.files();
        let found = file
            .and_then(|(path, area)| files.iter().position(|f| f.path == path && f.area == area));
        // When a staged file leaves its area, stay near the same spot.
        self.file_sel = found
            .unwrap_or(self.file_sel)
            .min(files.len().saturating_sub(1));
        self.load_diff();
    }

    /// Re-read the current repo synchronously (after a git operation).
    fn refresh_current(&mut self) {
        let Some(path) = self.current_repo().map(|r| r.path.clone()) else {
            return;
        };
        let status = git::status(&path).map_err(|e| e.to_string());
        self.apply_scan(vec![(path, status)]);
    }

    pub fn load_diff(&mut self) {
        let (Some(repo), Some(file)) = (self.current_repo(), self.current_file()) else {
            self.diff = None;
            self.rows.clear();
            return;
        };
        match git::diff(&repo.path, file) {
            // Same text as shown (the 3 s rescan, mostly): keep the parsed, highlighted diff.
            Ok(raw) if self.diff.is_some() && raw == self.diff_raw => {
                self.rows = self
                    .diff
                    .as_ref()
                    .map(|d| d.rows(self.split))
                    .unwrap_or_default();
            }
            Ok(raw) => {
                let parsed = diff::parse(&raw);
                // Show it plain now; colors follow from a worker (first use of a
                // language compiles its grammar, which can take a few hundred ms).
                let (tx, mut copy, path) = (
                    self.highlighted.0.clone(),
                    parsed.clone(),
                    file.path.clone(),
                );
                let key = raw.clone();
                std::thread::spawn(move || {
                    highlight::highlight(&mut copy, &path);
                    let _ = tx.send((key, copy));
                });
                self.diff_raw = raw;
                self.rows = parsed.rows(self.split);
                self.diff = Some(parsed);
            }
            Err(e) => {
                self.diff = None;
                self.rows.clear();
                self.message = e.to_string();
            }
        }
        self.cursor = self.cursor.min(self.rows.len().saturating_sub(1));
    }

    fn select_repo(&mut self, pos: usize) {
        if pos != self.repo_sel {
            self.repo_sel = pos;
            self.file_sel = 0;
            self.cursor = 0;
            self.scroll = 0;
            self.load_diff();
        }
    }

    fn select_file(&mut self, pos: usize) {
        if pos != self.file_sel {
            self.file_sel = pos;
            self.cursor = 0;
            self.scroll = 0;
            self.load_diff();
        }
    }

    // ---- keys ----------------------------------------------------------------------

    pub fn on_key(&mut self, key: KeyEvent) {
        // `std::mem::replace` takes the mode out so we can match on it by value.
        match std::mem::replace(&mut self.mode, Mode::Normal) {
            Mode::Normal => self.on_normal_key(key),
            Mode::Help => {}
            Mode::Commit(text) => self.on_input_key(key, text, Mode::Commit, Self::do_commit),
            Mode::Comment(text) => self.on_input_key(key, text, Mode::Comment, Self::add_comment),
            Mode::Confirm(pending, prompt) => {
                if key.code == KeyCode::Char('y') {
                    // Compute first: `self.report(self.x())` would borrow `self` mutably twice.
                    let result = run_pending(pending);
                    self.report(result);
                } else {
                    self.message = format!("cancelled: {prompt}");
                }
            }
        }
    }

    /// Shared line editor for the commit and comment prompts.
    fn on_input_key(
        &mut self,
        key: KeyEvent,
        mut text: String,
        wrap: fn(String) -> Mode,
        submit: fn(&mut Self, String) -> Result<String>,
    ) {
        match key.code {
            KeyCode::Esc => self.message = "cancelled".into(),
            KeyCode::Enter if key.modifiers.contains(KeyModifiers::ALT) => {
                text.push('\n');
                self.mode = wrap(text);
            }
            KeyCode::Enter => {
                let result = submit(self, text.trim().to_string());
                self.report(result);
            }
            KeyCode::Backspace => {
                text.pop();
                self.mode = wrap(text);
            }
            KeyCode::Char(c) => {
                text.push(c);
                self.mode = wrap(text);
            }
            _ => self.mode = wrap(text),
        }
    }

    fn report(&mut self, result: Result<String>) {
        self.message = match result {
            Ok(msg) => msg,
            Err(e) => format!("error: {e:#}"),
        };
        self.refresh_current();
    }

    fn on_normal_key(&mut self, key: KeyEvent) {
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        match key.code {
            KeyCode::Char('q') => self.quit = true,
            KeyCode::Char('c') if ctrl => self.quit = true,
            KeyCode::Char('?') => self.mode = Mode::Help,
            KeyCode::Tab | KeyCode::Char('l') | KeyCode::Right => self.focus_next(),
            KeyCode::BackTab | KeyCode::Esc | KeyCode::Char('h') | KeyCode::Left => {
                self.focus_prev()
            }
            KeyCode::Char('r') => {
                self.scanner.request();
                self.message = "refreshing…".into();
            }
            KeyCode::Char('a') => {
                self.hide_clean = !self.hide_clean;
                let repo = self.current_repo().map(|r| r.path.clone());
                self.reselect(repo, None);
            }
            KeyCode::Char('v') => {
                self.split = !self.split;
                self.load_diff();
            }
            KeyCode::Char('C') => {
                // Lock the repo now: the selection can move while the message is typed.
                self.commit_repo = self.current_repo().map(|r| r.path.clone());
                self.mode = Mode::Commit(String::new());
            }
            KeyCode::Char('S') => {
                let result = self.send_comments();
                self.report(result);
            }
            KeyCode::Char('X') => {
                self.comments.clear();
                self.message = "comments cleared".into();
            }
            _ => match self.focus {
                Focus::Repos => self.on_repos_key(key),
                Focus::Files => self.on_files_key(key),
                Focus::Diff => self.on_diff_key(key),
            },
        }
    }

    fn focus_next(&mut self) {
        self.focus = match self.focus {
            Focus::Repos => Focus::Files,
            _ => Focus::Diff,
        };
    }

    fn focus_prev(&mut self) {
        self.focus = match self.focus {
            Focus::Diff => Focus::Files,
            _ => Focus::Repos,
        };
    }

    fn on_repos_key(&mut self, key: KeyEvent) {
        let n = self.visible_repos().len();
        match key.code {
            KeyCode::Char('j') | KeyCode::Down => self.select_repo(step(self.repo_sel, 1, n)),
            KeyCode::Char('k') | KeyCode::Up => self.select_repo(step(self.repo_sel, -1, n)),
            KeyCode::Char('g') | KeyCode::Home => self.select_repo(0),
            KeyCode::Char('G') | KeyCode::End => self.select_repo(n.saturating_sub(1)),
            KeyCode::Enter => self.focus = Focus::Files,
            _ => {}
        }
    }

    fn on_files_key(&mut self, key: KeyEvent) {
        let n = self.files().len();
        match key.code {
            KeyCode::Char('j') | KeyCode::Down => self.select_file(step(self.file_sel, 1, n)),
            KeyCode::Char('k') | KeyCode::Up => self.select_file(step(self.file_sel, -1, n)),
            KeyCode::Char('g') | KeyCode::Home => self.select_file(0),
            KeyCode::Char('G') | KeyCode::End => self.select_file(n.saturating_sub(1)),
            KeyCode::Enter => self.focus = Focus::Diff,
            KeyCode::Char(' ') => {
                let result = self.toggle_file_stage();
                self.report(result);
            }
            KeyCode::Char('x') => self.ask_discard(false),
            KeyCode::Char('c') => self.mode = Mode::Comment(String::new()),
            _ => {}
        }
    }

    fn on_diff_key(&mut self, key: KeyEvent) {
        let n = self.rows.len();
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        match key.code {
            KeyCode::Char('j') | KeyCode::Down => self.cursor = step(self.cursor, 1, n),
            KeyCode::Char('k') | KeyCode::Up => self.cursor = step(self.cursor, -1, n),
            KeyCode::Char('d') if ctrl => self.cursor = step(self.cursor, 15, n),
            KeyCode::Char('u') if ctrl => self.cursor = step(self.cursor, -15, n),
            KeyCode::PageDown => self.cursor = step(self.cursor, 15, n),
            KeyCode::PageUp => self.cursor = step(self.cursor, -15, n),
            KeyCode::Char('g') | KeyCode::Home => self.cursor = 0,
            KeyCode::Char('G') | KeyCode::End => self.cursor = n.saturating_sub(1),
            KeyCode::Char('n') => self.jump_hunk(true),
            KeyCode::Char('p') => self.jump_hunk(false),
            KeyCode::Char(' ') => {
                let how = match self.current_file().map(|f| f.area) {
                    Some(Area::Staged) => Apply::Unstage,
                    _ => Apply::Stage,
                };
                let result = self.hunk_op(how);
                self.report(result);
            }
            KeyCode::Char('x') => self.ask_discard(true),
            KeyCode::Char('c') => self.mode = Mode::Comment(String::new()),
            _ => {}
        }
    }

    fn jump_hunk(&mut self, forward: bool) {
        let found = if forward {
            (self.cursor + 1..self.rows.len()).find(|&i| matches!(self.rows[i], Row::HunkHeader(_)))
        } else {
            (0..self.cursor)
                .rev()
                .find(|&i| matches!(self.rows[i], Row::HunkHeader(_)))
        };
        if let Some(i) = found {
            self.cursor = i;
        }
    }

    /// Open the confirm prompt for discarding the selected file, or the hunk under the cursor.
    fn ask_discard(&mut self, hunk: bool) {
        let prepared = self.repo_and_file().and_then(|(repo, file)| {
            if !hunk || file.area == Area::Untracked {
                let prompt = match file.area {
                    Area::Untracked => format!("delete untracked {}?", file.path),
                    _ => format!("discard all changes to {}?", file.path),
                };
                return Ok((Pending::DiscardFile { repo, file }, prompt));
            }
            if file.area == Area::Staged {
                bail!("unstage the hunk before discarding it");
            }
            let (n, patch) = self.cursor_hunk_patch()?;
            let prompt = format!("discard hunk {} of {}?", n + 1, file.path);
            Ok((
                Pending::DiscardHunk {
                    repo,
                    path: file.path,
                    patch,
                },
                prompt,
            ))
        });
        match prepared {
            Ok((pending, prompt)) => self.mode = Mode::Confirm(pending, prompt),
            Err(e) => self.message = format!("error: {e:#}"),
        }
    }

    /// Index and patch of the hunk under the diff cursor.
    fn cursor_hunk_patch(&self) -> Result<(usize, String)> {
        let hunk = self
            .rows
            .get(self.cursor)
            .context("no hunk under cursor")?
            .hunk();
        let patch = self
            .diff
            .as_ref()
            .and_then(|d| d.hunk_patch(&self.diff_raw, hunk))
            .context("no hunk under cursor")?;
        Ok((hunk, patch))
    }

    // ---- git operations ------------------------------------------------------------

    fn repo_and_file(&self) -> Result<(PathBuf, FileEntry)> {
        let repo = self.current_repo().context("no repo selected")?;
        let file = self.current_file().context("no file selected")?;
        Ok((repo.path.clone(), file.clone()))
    }

    fn toggle_file_stage(&mut self) -> Result<String> {
        let (repo, file) = self.repo_and_file()?;
        if file.area == Area::Staged {
            git::unstage_file(&repo, &file)?;
            Ok(format!("unstaged {}", file.path))
        } else {
            git::stage_file(&repo, &file.path)?;
            Ok(format!("staged {}", file.path))
        }
    }

    fn hunk_op(&mut self, how: Apply) -> Result<String> {
        let (repo, file) = self.repo_and_file()?;
        if file.area == Area::Untracked {
            // An untracked file is one "new file" hunk: stage the whole file.
            return self.toggle_file_stage();
        }
        let (hunk, patch) = self.cursor_hunk_patch()?;
        let verb = if matches!(how, Apply::Unstage) {
            "unstaged"
        } else {
            "staged"
        };
        git::apply_patch(&repo, &patch, how)?;
        Ok(format!("{verb} hunk {} of {}", hunk + 1, file.path))
    }

    fn do_commit(&mut self, message: String) -> Result<String> {
        if message.is_empty() {
            bail!("empty commit message");
        }
        let repo = self.commit_repo.take().context("no repo selected")?;
        git::commit(&repo, &message)
    }

    // ---- review comments -----------------------------------------------------------

    fn add_comment(&mut self, body: String) -> Result<String> {
        if body.is_empty() {
            bail!("empty comment");
        }
        let (repo, file) = self.repo_and_file()?;
        let (line, deleted, snippet) = match (self.focus, self.cursor_line()) {
            (Focus::Diff, Some(l)) => (
                l.new_no.or(l.old_no),
                l.new_no.is_none(),
                l.text.trim().to_string(),
            ),
            _ => (None, false, String::new()),
        };
        self.comments.push(Comment {
            repo,
            file: file.path,
            line,
            deleted,
            snippet,
            body,
        });
        Ok(format!(
            "{} comment(s) queued — S sends them to the agent",
            self.comments.len()
        ))
    }

    fn send_comments(&mut self) -> Result<String> {
        let repo = self
            .current_repo()
            .context("no repo selected")?
            .path
            .clone();
        let (mine, others): (Vec<Comment>, Vec<Comment>) = std::mem::take(&mut self.comments)
            .into_iter()
            .partition(|c| c.repo == repo);
        self.comments = others;
        if mine.is_empty() {
            bail!("no comments for this repo");
        }
        let text = format_review(&repo, &mine);
        let agent = match herdr::pick_agent(&repo) {
            Ok(a) if a.agent_status == "blocked" => {
                Err(anyhow::anyhow!("agent is waiting on a prompt"))
            }
            other => other,
        };
        let sent = agent.and_then(|a| herdr::send_text(&a.pane_id, &text).map(|_| a));
        match sent {
            Ok(agent) => {
                let _ = herdr::exec(&["agent", "focus", &agent.pane_id]);
                Ok(format!(
                    "sent {} comment(s) to {} ({})",
                    mine.len(),
                    agent.agent,
                    agent.pane_id
                ))
            }
            Err(e) => {
                self.comments.extend(mine); // keep them for another try
                Err(e)
            }
        }
    }
}

fn run_pending(pending: Pending) -> Result<String> {
    match pending {
        Pending::DiscardFile { repo, file } => {
            git::discard_file(&repo, &file)?;
            Ok(format!("discarded {}", file.path))
        }
        Pending::DiscardHunk { repo, path, patch } => {
            git::apply_patch(&repo, &patch, Apply::Discard)?;
            Ok(format!("discarded a hunk of {path}"))
        }
    }
}

/// Plain-text review the agent receives.
pub fn format_review(repo: &std::path::Path, comments: &[Comment]) -> String {
    let mut out = format!("Review comments on {}:\n", repo.display());
    for c in comments {
        let loc = match (c.line, c.deleted) {
            (Some(l), true) => format!("{} (deleted line, was line {l})", c.file),
            (Some(l), false) => format!("{}:{l}", c.file),
            (None, _) => c.file.clone(),
        };
        out.push_str(&format!("\n{loc}\n"));
        if !c.snippet.is_empty() {
            out.push_str(&format!("  > {}\n", c.snippet));
        }
        out.push_str(&format!("  {}\n", c.body.replace('\n', "\n  ")));
    }
    out
}

/// Move `pos` by `delta` within `0..len`, clamped.
fn step(pos: usize, delta: isize, len: usize) -> usize {
    if len == 0 {
        return 0;
    }
    pos.saturating_add_signed(delta).min(len - 1)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn review_text_lists_location_snippet_and_body() {
        let c = Comment {
            repo: "/r".into(),
            file: "src/a.rs".into(),
            line: Some(12),
            deleted: false,
            snippet: "let x = 1;".into(),
            body: "rename x\nplease".into(),
        };
        assert_eq!(
            format_review(std::path::Path::new("/r"), &[c]),
            "Review comments on /r:\n\nsrc/a.rs:12\n  > let x = 1;\n  rename x\n  please\n"
        );
    }

    #[test]
    fn step_clamps() {
        assert_eq!(step(0, -1, 5), 0);
        assert_eq!(step(4, 3, 5), 4);
        assert_eq!(step(2, 1, 0), 0);
    }
}
