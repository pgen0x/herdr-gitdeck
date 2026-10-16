//! Rendering. Left column: repos over files. Right: the diff (unified or split).
//! Colors come from the terminal palette so the pane follows the user's theme.

use crate::app::{App, Focus, Mode};
use crate::diff::{Kind, Row};
use crate::git::{Area, FileEntry};
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Color, Modifier, Style, Stylize};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Clear, List, ListItem, ListState, Paragraph, Wrap};
use ratatui::Frame;

const ACCENT: Color = Color::Cyan;
const DIM: Color = Color::DarkGray;
const ADD: Color = Color::Green;
const DEL: Color = Color::Red;
const WARN: Color = Color::Yellow;
const CURSOR_BG: Color = Color::Indexed(237);
/// Row tints for highlighted add/del lines: the foreground is busy showing syntax.
const ADD_BG: Color = Color::Rgb(28, 52, 34);
const DEL_BG: Color = Color::Rgb(62, 30, 32);
/// Below this many columns the layout stacks vertically.
const NARROW: u16 = 80;

pub fn draw(f: &mut Frame, app: &mut App) {
    let [main, status] =
        Layout::vertical([Constraint::Fill(1), Constraint::Length(1)]).areas(f.area());
    // Narrow pane: stack the lists above the diff instead of beside it.
    let [left, right] = if main.width < NARROW {
        // The focused side gets the room: lists shrink while reading a diff.
        let lists = if app.focus == Focus::Diff { 30 } else { 55 };
        Layout::vertical([Constraint::Percentage(lists), Constraint::Fill(1)]).areas(main)
    } else {
        let left_width = (main.width * 2 / 5).clamp(30, 56);
        Layout::horizontal([Constraint::Length(left_width), Constraint::Fill(1)]).areas(main)
    };
    let [repos, files] =
        Layout::vertical([Constraint::Percentage(45), Constraint::Fill(1)]).areas(left);

    let repos_offset = draw_repos(f, app, repos);
    let (files_offset, file_rows) = draw_files(f, app, files);
    draw_diff(f, app, right);
    app.hits = crate::app::Hitmap {
        repos,
        repos_offset,
        files,
        files_offset,
        file_rows,
        diff: right,
    };
    draw_status(f, app, status);
    draw_popup(f, app);
}

/// Focused panel: thick accent border and a ▸ marker, so focus moves are obvious.
fn panel(title: Line<'_>, focused: bool) -> Block<'_> {
    let (color, border, marker) = if focused {
        (ACCENT, BorderType::Thick, "▸")
    } else {
        (DIM, BorderType::Rounded, "")
    };
    let mut title = title;
    title
        .spans
        .insert(0, Span::styled(marker, Style::new().fg(ACCENT).bold()));
    Block::bordered()
        .border_type(border)
        .border_style(Style::new().fg(color))
        .title(title)
}

/// Selection highlight: solid in the focused list, a quiet accent elsewhere.
fn selection(focused: bool) -> Style {
    if focused {
        Style::new().bg(CURSOR_BG).bold()
    } else {
        Style::new().fg(ACCENT)
    }
}

// ---- repos ---------------------------------------------------------------------------

/// Returns the list's scroll offset.
fn draw_repos(f: &mut Frame, app: &App, area: Rect) -> usize {
    let vis = app.visible_repos();
    let dirty = vis
        .iter()
        .filter(|&&i| app.repos[i].ok_status().is_some_and(|s| s.is_dirty()))
        .count();
    let filter = if app.hide_clean {
        " · changed only"
    } else {
        ""
    };
    let title = Line::from(vec![
        " repos ".bold(),
        format!("{} · {dirty} dirty{filter} ", vis.len()).fg(DIM),
    ]);
    let items: Vec<ListItem> = vis
        .iter()
        .map(|&i| {
            let r = &app.repos[i];
            let mut spans = Vec::new();
            match &r.status {
                None => spans.push("◌ ".fg(DIM)),
                Some(Err(_)) => spans.push("✗ ".fg(DEL)),
                Some(Ok(s)) if s.is_dirty() => spans.push("● ".fg(WARN)),
                Some(Ok(_)) => spans.push("● ".fg(ADD)),
            }
            spans.push(Span::raw(r.name.clone()));
            // Counts before the branch: in a narrow pane the branch is what gets cut.
            if let Some(s) = r.ok_status() {
                spans.extend(counts(
                    s.count(Area::Staged),
                    s.count(Area::Unstaged),
                    s.count(Area::Untracked),
                ));
                if s.ahead > 0 {
                    spans.push(format!(" ↑{}", s.ahead).fg(ACCENT));
                }
                if s.behind > 0 {
                    spans.push(format!(" ↓{}", s.behind).fg(DEL));
                }
                spans.push(format!("  {}", s.branch).fg(DIM));
            }
            ListItem::new(Line::from(spans))
        })
        .collect();
    let empty = items.is_empty();
    let list = List::new(items)
        .block(panel(title, app.focus == Focus::Repos))
        .highlight_style(selection(app.focus == Focus::Repos));
    let mut state = ListState::default().with_selected((!empty).then_some(app.repo_sel));
    f.render_stateful_widget(list, area, &mut state);
    if empty {
        let msg = if app.repos.is_empty() {
            format!("no git repos under {}", app.root.display())
        } else {
            "all repos clean (a: show all)".into()
        };
        f.render_widget(Paragraph::new(msg.fg(DIM)), inner(area));
    }
    state.offset()
}

fn counts(staged: usize, unstaged: usize, untracked: usize) -> Vec<Span<'static>> {
    let mut v = Vec::new();
    if staged > 0 {
        v.push(format!(" +{staged}").fg(ADD));
    }
    if unstaged > 0 {
        v.push(format!(" ~{unstaged}").fg(WARN));
    }
    if untracked > 0 {
        v.push(format!(" ?{untracked}").fg(Color::Magenta));
    }
    v
}

fn inner(area: Rect) -> Rect {
    Rect {
        x: area.x + 2,
        y: area.y + 1,
        width: area.width.saturating_sub(4),
        height: 1,
    }
}

// ---- files ---------------------------------------------------------------------------

fn area_label(area: Area) -> (&'static str, Color) {
    match area {
        Area::Staged => ("STAGED", ADD),
        Area::Unstaged => ("CHANGES", WARN),
        Area::Untracked => ("UNTRACKED", Color::Magenta),
    }
}

/// Returns the list's scroll offset and the file index of each list row.
fn draw_files(f: &mut Frame, app: &App, area: Rect) -> (usize, Vec<Option<usize>>) {
    let repo = app.current_repo();
    let title = Line::from(vec![
        " files ".bold(),
        repo.map(|r| format!("{} ", r.name))
            .unwrap_or_default()
            .fg(DIM),
    ]);
    let files = app.files();
    // Section headers are interleaved, so the list index of a file is shifted.
    let mut items = Vec::new();
    let mut rows = Vec::new();
    let mut selected = None;
    let mut last_area = None;
    for (i, file) in files.iter().enumerate() {
        if last_area != Some(file.area) {
            let (label, color) = area_label(file.area);
            let n = files.iter().filter(|f| f.area == file.area).count();
            items.push(ListItem::new(Line::from(vec![
                label.fg(color).bold(),
                format!(" {n}").fg(DIM),
            ])));
            rows.push(None);
            last_area = Some(file.area);
        }
        if i == app.file_sel {
            selected = Some(items.len());
        }
        items.push(ListItem::new(file_line(app, file)));
        rows.push(Some(i));
    }
    let list = List::new(items)
        .block(panel(title, app.focus == Focus::Files))
        .highlight_style(selection(app.focus == Focus::Files));
    let mut state = ListState::default().with_selected(selected);
    f.render_stateful_widget(list, area, &mut state);
    if let Some(Some(Err(e))) = repo.map(|r| &r.status) {
        f.render_widget(
            Paragraph::new(e.as_str().fg(DEL)).wrap(Wrap { trim: true }),
            inner(area),
        );
    } else if files.is_empty() && repo.is_some() {
        f.render_widget(Paragraph::new("working tree clean".fg(DIM)), inner(area));
    }
    (state.offset(), rows)
}

fn file_line(app: &App, file: &FileEntry) -> Line<'static> {
    let (_, color) = area_label(file.area);
    let (dir, name) = match file.path.rsplit_once('/') {
        Some((d, n)) => (format!("{d}/"), n.to_string()),
        None => (String::new(), file.path.clone()),
    };
    let mut spans = vec![
        format!(" {} ", file.code).fg(color),
        name.into(),
        format!(" {dir}").fg(DIM),
    ];
    let n = app.comments.iter().filter(|c| c.file == file.path).count();
    if n > 0 {
        spans.push(format!(" ●{n}").fg(WARN));
    }
    Line::from(spans)
}

// ---- diff ----------------------------------------------------------------------------

fn draw_diff(f: &mut Frame, app: &mut App, area: Rect) {
    let file = app.current_file().cloned();
    let mode = if app.split { "split" } else { "unified" };
    let title = match &file {
        Some(file) => {
            let (label, color) = area_label(file.area);
            Line::from(vec![
                format!(" {} ", file.path).bold(),
                label.to_lowercase().fg(color),
                format!(" · {mode} ").fg(DIM),
            ])
        }
        None => Line::from(" diff ".bold()),
    };
    let block = panel(title, app.focus == Focus::Diff);
    let body = block.inner(area);
    f.render_widget(block, area);

    let Some(diff) = app.diff.as_ref() else {
        f.render_widget(
            Paragraph::new("select a file to see its diff".fg(DIM)),
            body,
        );
        return;
    };
    if diff.hunks.is_empty() {
        let mut lines: Vec<Line> = diff
            .header
            .iter()
            .map(|l| Line::from(l.clone().fg(DIM)))
            .collect();
        if lines.is_empty() {
            lines.push(Line::from(
                "no textual diff (nested repo, folder or empty file)".fg(DIM),
            ));
        }
        f.render_widget(Paragraph::new(lines), body);
        return;
    }

    // Keep the cursor on screen.
    let height = body.height as usize;
    if app.cursor < app.scroll {
        app.scroll = app.cursor;
    } else if height > 0 && app.cursor >= app.scroll + height {
        app.scroll = app.cursor + 1 - height;
    }
    let visible = app.rows.iter().enumerate().skip(app.scroll).take(height);
    let path = file.map(|f| f.path).unwrap_or_default();

    if app.split {
        let [l, divider, r] = Layout::horizontal([
            Constraint::Fill(1),
            Constraint::Length(1),
            Constraint::Fill(1),
        ])
        .areas(body);
        let bar = vec![Line::from("│".fg(DIM)); divider.height as usize];
        f.render_widget(Paragraph::new(bar), divider);
        let (mut left, mut right) = (Vec::new(), Vec::new());
        for (i, row) in visible {
            let cur = i == app.cursor && app.focus == Focus::Diff;
            match *row {
                Row::HunkHeader(h) => {
                    let header = hunk_line(&diff.hunks[h].header, cur);
                    left.push(header.clone());
                    right.push(header);
                }
                Row::Pair { hunk, old, new } => {
                    let lines = &diff.hunks[hunk].lines;
                    left.push(side(app, &path, old.map(|i| &lines[i]), true, cur));
                    right.push(side(app, &path, new.map(|i| &lines[i]), false, cur));
                }
                Row::Line { .. } => {}
            }
        }
        f.render_widget(Paragraph::new(left), l);
        f.render_widget(Paragraph::new(right), r);
    } else {
        let mut lines = Vec::new();
        for (i, row) in visible {
            let cur = i == app.cursor && app.focus == Focus::Diff;
            match *row {
                Row::HunkHeader(h) => lines.push(hunk_line(&diff.hunks[h].header, cur)),
                Row::Line { hunk, idx } => {
                    lines.push(unified(app, &path, &diff.hunks[hunk].lines[idx], cur))
                }
                Row::Pair { .. } => {}
            }
        }
        f.render_widget(Paragraph::new(lines), body);
    }
}

fn hunk_line(header: &str, cur: bool) -> Line<'static> {
    let line = Line::from(header.to_string().fg(ACCENT));
    if cur {
        line.bg(CURSOR_BG)
    } else {
        line
    }
}

fn kind_style(kind: Kind) -> (char, Style) {
    match kind {
        Kind::Add => ('+', Style::new().fg(ADD)),
        Kind::Del => ('-', Style::new().fg(DEL)),
        Kind::Context => (' ', Style::new()),
        Kind::Meta => (' ', Style::new().fg(DIM).italic()),
    }
}

/// Sign plus text: syntax colors on a tinted row when highlighted, flat diff colors otherwise.
fn content(line: &crate::diff::Line, cur: bool) -> Vec<Span<'static>> {
    let (sign, style) = kind_style(line.kind);
    if line.spans.is_empty() {
        return vec![Span::styled(format!("{sign}{}", line.text), style)];
    }
    // The cursor row keeps its own background, so skip the tint there.
    let bg = match line.kind {
        Kind::Add if !cur => Some(ADD_BG),
        Kind::Del if !cur => Some(DEL_BG),
        _ => None,
    };
    let tint = |s: Style| bg.map_or(s, |c| s.bg(c));
    let mut out = vec![Span::styled(sign.to_string(), tint(style.bold()))];
    out.extend(
        line.spans
            .iter()
            .map(|(fg, text)| Span::styled(text.clone(), tint(Style::new().fg(*fg)))),
    );
    out
}

fn gutter_no(n: Option<u32>) -> String {
    n.map_or("    ".into(), |n| format!("{n:>4}"))
}

fn marker(app: &App, path: &str, line: &crate::diff::Line) -> Span<'static> {
    if app.has_comment(path, line.new_no.or(line.old_no)) {
        "●".fg(WARN)
    } else {
        " ".into()
    }
}

fn unified(app: &App, path: &str, line: &crate::diff::Line, cur: bool) -> Line<'static> {
    let mut spans = vec![
        marker(app, path, line),
        format!("{} {} ", gutter_no(line.old_no), gutter_no(line.new_no)).fg(DIM),
    ];
    spans.extend(content(line, cur));
    let out = Line::from(spans);
    if cur {
        out.bg(CURSOR_BG)
    } else {
        out
    }
}

fn side(
    app: &App,
    path: &str,
    line: Option<&crate::diff::Line>,
    old: bool,
    cur: bool,
) -> Line<'static> {
    let out = match line {
        None => Line::from("".fg(DIM)),
        Some(line) => {
            let no = if old { line.old_no } else { line.new_no };
            let mut spans = vec![
                marker(app, path, line),
                format!("{} ", gutter_no(no)).fg(DIM),
            ];
            spans.extend(content(line, cur));
            Line::from(spans)
        }
    };
    if cur {
        out.bg(CURSOR_BG)
    } else {
        out
    }
}

// ---- status bar & popups -------------------------------------------------------------

fn draw_status(f: &mut Frame, app: &App, area: Rect) {
    let hints = match app.focus {
        Focus::Repos => "j/k move · enter files · a changed-only · r refresh · C commit",
        Focus::Files => "space stage/unstage · x discard · c comment · enter diff · C commit",
        Focus::Diff => "space stage hunk · x discard hunk · n/p hunk · c comment · v split",
    };
    let left = if app.message.is_empty() {
        hints.fg(DIM)
    } else {
        app.message.clone().into()
    };
    let n = app.comments.len();
    let right = Line::from(vec![
        if n > 0 {
            format!("{n} comment(s) · S send  ").fg(WARN)
        } else {
            "".into()
        },
        "  ? help ".fg(DIM),
    ]);
    let width = right.width() as u16;
    let [l, r] = Layout::horizontal([Constraint::Fill(1), Constraint::Length(width)]).areas(area);
    f.render_widget(Paragraph::new(Line::from(left)), l);
    f.render_widget(Paragraph::new(right), r);
}

fn draw_popup(f: &mut Frame, app: &App) {
    let (title, body, height): (String, Vec<Line>, u16) = match &app.mode {
        Mode::Normal => return,
        Mode::Help => (
            " keys ".into(),
            HELP.iter().map(|l| Line::from(*l)).collect(),
            HELP.len() as u16 + 2,
        ),
        Mode::Commit(text) => (
            format!(
                " commit to {} · enter commit · alt+enter newline · esc ",
                app.commit_repo
                    .as_ref()
                    .and_then(|p| p.file_name())
                    .map_or("?".into(), |n| n.to_string_lossy())
            ),
            input(text),
            8,
        ),
        Mode::Comment(text) => (" comment · enter to queue · esc ".into(), input(text), 6),
        Mode::Confirm(_, prompt) => (
            " confirm ".into(),
            vec![
                Line::from(prompt.clone().bold()),
                Line::from("y = yes, any other key = no".fg(DIM)),
            ],
            4,
        ),
    };
    let area = f.area();
    let width = area.width.saturating_sub(8).min(72);
    let rect = Rect {
        x: area.x + (area.width - width) / 2,
        y: area.y + area.height.saturating_sub(height) / 3,
        width,
        height: height.min(area.height),
    };
    f.render_widget(Clear, rect);
    let block = Block::bordered()
        .border_type(BorderType::Rounded)
        .border_style(ACCENT)
        .title(title);
    f.render_widget(
        Paragraph::new(body).wrap(Wrap { trim: false }).block(block),
        rect,
    );
}

fn input(text: &str) -> Vec<Line<'static>> {
    let mut lines: Vec<Line> = text
        .split('\n')
        .map(|l| Line::from(l.to_string()))
        .collect();
    if let Some(last) = lines.last_mut() {
        last.push_span(Span::styled(
            "█",
            Style::new().fg(ACCENT).add_modifier(Modifier::SLOW_BLINK),
        ));
    }
    lines
}

const HELP: &[&str] = &[
    " tab/enter   next panel · esc/h previous panel",
    " j/k g/G     move · top · bottom (ctrl+d/u page)",
    " a           show only repos with changes",
    " r           rescan all repos now",
    " space       stage/unstage file (files) or hunk (diff)",
    " x           discard file or hunk (asks first)",
    " n/p         next / previous hunk",
    " v           unified ↔ split diff",
    " c           comment on file or line",
    " S           send this repo's comments to the agent",
    " X           clear all queued comments",
    " C           commit staged changes",
    " q           quit",
];
