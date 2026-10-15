//! Plugin actions (`--action toggle|open|close`). herdr runs them with the plugin
//! root as cwd and the focused workspace in `HERDR_PLUGIN_CONTEXT_JSON`.

use crate::herdr;
use anyhow::{bail, Context, Result};
use serde_json::Value;

const PLUGIN_ID: &str = "pgen0x.gitdeck";
/// herdr labels a plugin pane with its entrypoint title; that is how we find ours.
const PANE_LABEL: &str = "gitdeck";

pub fn run(action: &str) -> Result<()> {
    let raw = std::env::var("HERDR_PLUGIN_CONTEXT_JSON").context("no plugin context")?;
    let ctx: Value = serde_json::from_str(&raw)?;
    let field = |k: &str| ctx[k].as_str().unwrap_or_default().to_string();
    let workspace = field("workspace_id");
    let ours: Vec<herdr::Pane> = herdr::panes(&workspace)?
        .into_iter()
        .filter(|p| p.label.as_deref() == Some(PANE_LABEL))
        .collect();

    match action {
        "close" => close(&ours),
        "open" if !ours.is_empty() => Ok(()),
        "open" => open(&ctx, &workspace),
        "toggle" if ours.is_empty() => open(&ctx, &workspace),
        "toggle" => close(&ours),
        other => bail!("unknown action {other}"),
    }
}

fn close(panes: &[herdr::Pane]) -> Result<()> {
    for p in panes {
        herdr::exec(&["pane", "close", &p.pane_id])?;
    }
    Ok(())
}

fn open(ctx: &Value, workspace: &str) -> Result<()> {
    let focused = ctx["focused_pane_id"].as_str().context("no focused pane")?;
    // The context cwd is the pane's launch cwd; the live one comes from the pane list.
    let live_cwd = herdr::panes(workspace)?
        .into_iter()
        .find(|p| p.pane_id == focused)
        .and_then(|p| p.foreground_cwd);
    let cwd = live_cwd
        .or_else(|| ctx["focused_pane_cwd"].as_str().map(String::from))
        .or_else(|| ctx["workspace_cwd"].as_str().map(String::from))
        .context("no cwd to open in")?;
    herdr::exec(&[
        "plugin",
        "pane",
        "open",
        "--plugin",
        PLUGIN_ID,
        "--entrypoint",
        "pane",
        "--placement",
        "split",
        "--direction",
        "right",
        "--target-pane",
        focused,
        "--cwd",
        &cwd,
        "--no-focus",
    ])?;
    // Workaround (herdr 0.9.3): a pane opened with `--focus` keeps its pre-layout pty size
    // (34 cols in a 73-col pane) until focus next changes. Opening unfocused and then moving
    // focus ourselves makes herdr send the real size. The new pane sits right of `focused`.
    herdr::exec(&["pane", "focus", "--direction", "right", "--pane", focused])
}
