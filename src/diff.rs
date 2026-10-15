//! Parse a single-file unified diff into hunks, and rebuild one hunk as a patch
//! that `git apply` accepts (that is how hunk staging works).

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Context,
    Add,
    Del,
    /// "\ No newline at end of file"
    Meta,
}

#[derive(Clone, Debug)]
pub struct Line {
    pub kind: Kind,
    /// Text without the leading ' ', '+' or '-'.
    pub text: String,
    pub old_no: Option<u32>,
    pub new_no: Option<u32>,
    /// Syntax-colored pieces of `text`; empty means draw it plain.
    pub spans: Vec<(ratatui::style::Color, String)>,
}

#[derive(Clone, Debug)]
pub struct Hunk {
    /// The full "@@ -a,b +c,d @@ ..." line.
    pub header: String,
    pub lines: Vec<Line>,
}

#[derive(Clone, Debug, Default)]
pub struct FileDiff {
    /// Everything before the first hunk: "diff --git", "index", "---", "+++".
    pub header: Vec<String>,
    pub hunks: Vec<Hunk>,
}

/// One rendered row of the diff view. Indices point into `FileDiff::hunks[..].lines`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Row {
    HunkHeader(usize),
    /// Unified view: one line.
    Line {
        hunk: usize,
        idx: usize,
    },
    /// Split view: old line on the left, new line on the right.
    Pair {
        hunk: usize,
        old: Option<usize>,
        new: Option<usize>,
    },
}

impl Row {
    pub fn hunk(&self) -> usize {
        match *self {
            Row::HunkHeader(h) | Row::Line { hunk: h, .. } | Row::Pair { hunk: h, .. } => h,
        }
    }
}

/// Parse "@@ -a,b +c,d @@" into the two start line numbers.
fn hunk_starts(header: &str) -> (u32, u32) {
    let mut parts = header.split_whitespace().skip(1);
    let start = |s: Option<&str>| {
        s.and_then(|s| s.get(1..)?.split(',').next()?.parse().ok())
            .unwrap_or(0)
    };
    (start(parts.next()), start(parts.next()))
}

pub fn parse(text: &str) -> FileDiff {
    let mut diff = FileDiff::default();
    let (mut old_no, mut new_no) = (0, 0);
    for raw in text.lines() {
        if raw.starts_with("@@") {
            (old_no, new_no) = hunk_starts(raw);
            diff.hunks.push(Hunk {
                header: raw.to_string(),
                lines: Vec::new(),
            });
            continue;
        }
        // `last_mut()` is None until the first hunk: those lines are the file header.
        let Some(hunk) = diff.hunks.last_mut() else {
            diff.header.push(raw.to_string());
            continue;
        };
        let (kind, body) = match raw.chars().next() {
            Some('+') => (Kind::Add, &raw[1..]),
            Some('-') => (Kind::Del, &raw[1..]),
            Some('\\') => (Kind::Meta, raw),
            Some(' ') => (Kind::Context, &raw[1..]),
            _ => (Kind::Context, raw), // an empty context line can lose its space
        };
        let line = Line {
            kind,
            text: body.replace('\t', "    "),
            old_no: matches!(kind, Kind::Context | Kind::Del).then_some(old_no),
            new_no: matches!(kind, Kind::Context | Kind::Add).then_some(new_no),
            spans: Vec::new(),
        };
        if line.old_no.is_some() {
            old_no += 1;
        }
        if line.new_no.is_some() {
            new_no += 1;
        }
        hunk.lines.push(line);
    }
    diff
}

impl FileDiff {
    /// A patch holding the file header plus hunk `i`, copied byte for byte from the
    /// raw diff: `\r\n` endings and tabs must survive or `git apply` rejects the patch.
    pub fn hunk_patch(&self, original: &str, i: usize) -> Option<String> {
        let mut out = String::new();
        let mut current: Option<usize> = None;
        for raw in original.split_inclusive('\n') {
            if raw.starts_with("@@") {
                current = Some(current.map_or(0, |c| c + 1));
            }
            if current.is_none() || current == Some(i) {
                out.push_str(raw);
            }
        }
        (current >= Some(i) && !self.hunks.is_empty()).then_some(out)
    }

    /// Rows for the view. `split` pairs deletions with the additions that follow them.
    pub fn rows(&self, split: bool) -> Vec<Row> {
        let mut rows = Vec::new();
        for (h, hunk) in self.hunks.iter().enumerate() {
            rows.push(Row::HunkHeader(h));
            if !split {
                rows.extend((0..hunk.lines.len()).map(|idx| Row::Line { hunk: h, idx }));
                continue;
            }
            let mut i = 0;
            while i < hunk.lines.len() {
                match hunk.lines[i].kind {
                    Kind::Context | Kind::Meta => {
                        rows.push(Row::Pair {
                            hunk: h,
                            old: Some(i),
                            new: Some(i),
                        });
                        i += 1;
                    }
                    Kind::Add | Kind::Del => {
                        // Collect a block of deletions then additions and zip them side by side.
                        let dels: Vec<usize> = (i..hunk.lines.len())
                            .take_while(|&j| hunk.lines[j].kind == Kind::Del)
                            .collect();
                        i += dels.len();
                        let adds: Vec<usize> = (i..hunk.lines.len())
                            .take_while(|&j| hunk.lines[j].kind == Kind::Add)
                            .collect();
                        i += adds.len();
                        for k in 0..dels.len().max(adds.len()) {
                            let (old, new) = (dels.get(k).copied(), adds.get(k).copied());
                            rows.push(Row::Pair { hunk: h, old, new });
                        }
                    }
                }
            }
        }
        rows
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // `concat!` keeps each line's leading space (a `\` line continuation would eat it).
    const SAMPLE: &str = concat!(
        "diff --git a/f.txt b/f.txt\n",
        "index 111..222 100644\n",
        "--- a/f.txt\n",
        "+++ b/f.txt\n",
        "@@ -1,3 +1,3 @@\n",
        " one\n",
        "-two\n",
        "+TWO\n",
        " three\n",
        "@@ -10,2 +10,3 @@ fn x\n",
        " ten\n",
        "+\tadded\n",
        " eleven\n",
    );

    #[test]
    fn parses_hunks_and_line_numbers() {
        let d = parse(SAMPLE);
        assert_eq!(d.header.len(), 4);
        assert_eq!(d.hunks.len(), 2);
        let h0 = &d.hunks[0].lines;
        assert_eq!(
            (h0[1].kind, h0[1].old_no, h0[1].new_no),
            (Kind::Del, Some(2), None)
        );
        assert_eq!(
            (h0[2].kind, h0[2].old_no, h0[2].new_no),
            (Kind::Add, None, Some(2))
        );
        assert_eq!(h0[3].new_no, Some(3));
        assert_eq!(d.hunks[1].lines[1].new_no, Some(11));
    }

    #[test]
    fn hunk_patch_keeps_header_and_one_hunk() {
        let d = parse(SAMPLE);
        let p = d.hunk_patch(SAMPLE, 1).unwrap();
        assert!(p.starts_with("diff --git a/f.txt b/f.txt\n"));
        assert!(p.contains("@@ -10,2 +10,3 @@ fn x\n ten\n+\tadded\n eleven\n"));
        assert!(!p.contains("TWO"));
        assert!(d.hunk_patch(SAMPLE, 2).is_none());
    }

    #[test]
    fn split_rows_pair_del_with_add() {
        let d = parse(SAMPLE);
        let rows = d.rows(true);
        assert_eq!(
            rows[2],
            Row::Pair {
                hunk: 0,
                old: Some(1),
                new: Some(2)
            }
        );
        assert_eq!(rows.len(), 1 + 3 + 1 + 3);
    }
}
