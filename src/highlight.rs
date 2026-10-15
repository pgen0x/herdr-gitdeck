//! Syntax highlighting for diff lines, via syntect with bat's syntax/theme bundle (two-face).
//! Runs once per diff load, never per frame.

use crate::diff::{FileDiff, Kind};
use ratatui::style::Color;
use std::path::Path;
use std::sync::OnceLock;
use syntect::easy::HighlightLines;
use syntect::highlighting::Theme;
use syntect::parsing::{SyntaxReference, SyntaxSet};
use two_face::theme::EmbeddedThemeName;

/// Past this many lines a diff is shown plain: highlighting would stall the UI.
const MAX_LINES: usize = 4000;

struct Assets {
    syntaxes: SyntaxSet,
    theme: Theme,
}

/// Loading the bundle takes ~100 ms, so do it once, on first use.
fn assets() -> &'static Assets {
    static ASSETS: OnceLock<Assets> = OnceLock::new();
    ASSETS.get_or_init(|| Assets {
        syntaxes: two_face::syntax::extra_newlines(),
        theme: two_face::theme::extra()
            .get(EmbeddedThemeName::OneHalfDark)
            .clone(),
    })
}

/// Start loading in the background so the first diff doesn't pay for it.
pub fn warm_up() {
    std::thread::spawn(assets);
}

fn syntax_for<'a>(set: &'a SyntaxSet, path: &str) -> Option<&'a SyntaxReference> {
    let p = Path::new(path);
    let ext = p.extension().and_then(|e| e.to_str());
    let name = p.file_name().and_then(|n| n.to_str()).unwrap_or(path);
    // Extension first (`rs`, `tsx`), then whole names like `Dockerfile` or `Makefile`.
    ext.and_then(|e| set.find_syntax_by_extension(e))
        .or_else(|| set.find_syntax_by_extension(name))
}

/// Fill `Line::spans` for every line of `diff`. Lines stay plain when the file type is unknown.
pub fn highlight(diff: &mut FileDiff, path: &str) {
    let total: usize = diff.hunks.iter().map(|h| h.lines.len()).sum();
    if total > MAX_LINES {
        return;
    }
    let Assets { syntaxes, theme } = assets();
    let Some(syntax) = syntax_for(syntaxes, path) else {
        return;
    };

    for hunk in &mut diff.hunks {
        // Old and new sides are separate texts, each with its own parser state.
        // ponytail: state starts fresh per hunk, so a hunk opening inside a block comment
        // or multi-line string colors wrong until it closes; fix by parsing the whole file.
        let mut old = HighlightLines::new(syntax, theme);
        let mut new = HighlightLines::new(syntax, theme);
        for line in &mut hunk.lines {
            let text = format!("{}\n", line.text);
            let ranges = match line.kind {
                Kind::Meta => continue,
                Kind::Del => old.highlight_line(&text, syntaxes),
                Kind::Add => new.highlight_line(&text, syntaxes),
                Kind::Context => {
                    let _ = old.highlight_line(&text, syntaxes); // keep the old side in step
                    new.highlight_line(&text, syntaxes)
                }
            };
            let Ok(ranges) = ranges else { continue };
            line.spans = ranges
                .into_iter()
                .map(|(style, piece)| {
                    let c = style.foreground;
                    (
                        Color::Rgb(c.r, c.g, c.b),
                        piece.trim_end_matches('\n').to_string(),
                    )
                })
                .filter(|(_, piece)| !piece.is_empty())
                .collect();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn colors_known_files_and_skips_unknown() {
        let raw = "--- a/x\n+++ b/x\n@@ -1,1 +1,1 @@\n-let a = 1;\n+let b = \"two\";\n";
        let mut rs = crate::diff::parse(raw);
        highlight(&mut rs, "src/main.rs");
        let added = &rs.hunks[0].lines[1];
        let joined: String = added.spans.iter().map(|(_, s)| s.as_str()).collect();
        assert_eq!(joined, added.text);
        let colors: std::collections::HashSet<_> = added.spans.iter().map(|(c, _)| *c).collect();
        assert!(
            colors.len() > 1,
            "keyword, string and punctuation should differ"
        );

        let mut unknown = crate::diff::parse(raw);
        highlight(&mut unknown, "notes.unknown-ext");
        assert!(unknown.hunks[0].lines[1].spans.is_empty());
    }
}
