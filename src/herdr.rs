//! Talking to herdr: the CLI for reads, the socket for sending text to an agent.

use anyhow::{bail, Context, Result};
use serde::Deserialize;
use serde_json::{json, Value};
use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::path::Path;
use std::process::Command;
use std::time::Duration;

/// herdr gives up on a request after 5 s; wait a little longer for its answer.
const SOCKET_TIMEOUT: Duration = Duration::from_secs(7);

#[derive(Clone, Debug, Deserialize)]
pub struct Agent {
    pub pane_id: String,
    pub workspace_id: String,
    #[serde(default)]
    pub agent: String,
    #[serde(default)]
    pub agent_status: String,
    #[serde(default)]
    pub foreground_cwd: String,
}

#[derive(Clone, Debug, Deserialize)]
pub struct Pane {
    pub pane_id: String,
    #[serde(default)]
    pub label: Option<String>,
    #[serde(default)]
    pub foreground_cwd: Option<String>,
}

pub fn bin() -> String {
    std::env::var("HERDR_BIN_PATH").unwrap_or_else(|_| "herdr".into())
}

/// Run `herdr <args>` and return the `.result` of its JSON reply.
pub fn cli(args: &[&str]) -> Result<Value> {
    let out = Command::new(bin())
        .args(args)
        .output()
        .context("failed to run herdr")?;
    if !out.status.success() {
        bail!(
            "herdr {}: {}",
            args.join(" "),
            String::from_utf8_lossy(&out.stderr).trim()
        );
    }
    let mut reply: Value =
        serde_json::from_slice(&out.stdout).context("herdr reply is not JSON")?;
    Ok(reply["result"].take())
}

pub fn agents() -> Result<Vec<Agent>> {
    let mut res = cli(&["agent", "list"])?;
    Ok(serde_json::from_value(res["agents"].take())?)
}

pub fn panes(workspace: &str) -> Result<Vec<Pane>> {
    let mut res = cli(&["pane", "list", "--workspace", workspace])?;
    Ok(serde_json::from_value(res["panes"].take())?)
}

/// Pick the agent to send a review to: same workspace, preferring one working in `repo`.
pub fn pick_agent(repo: &Path) -> Result<Agent> {
    let workspace = std::env::var("HERDR_WORKSPACE_ID").context("not running inside herdr")?;
    let candidates: Vec<Agent> = agents()?
        .into_iter()
        .filter(|a| a.workspace_id == workspace)
        .collect();
    // ponytail: no picker; prefer the agent whose cwd is inside the repo, else the first one.
    candidates
        .iter()
        .find(|a| Path::new(&a.foreground_cwd).starts_with(repo))
        .or(candidates.first())
        .cloned()
        .context("no agent in this workspace")
}

/// Keep only printable text, newlines and tabs. ESC could close the bracketed paste early,
/// and other controls (^C, ^D, CR, C1 CSI) act as keys if the paste lands raw. Review
/// snippets come from repo content, which is untrusted.
fn sanitize_paste(text: &str) -> String {
    text.chars()
        .filter(|&c| !c.is_control() || c == '\n' || c == '\t')
        .collect()
}

/// Paste `text` into the agent's input without pressing Enter.
pub fn send_text(pane_id: &str, text: &str) -> Result<()> {
    let socket = std::env::var("HERDR_SOCKET_PATH").context("HERDR_SOCKET_PATH not set")?;
    let clean = sanitize_paste(text);
    let request = json!({
        "id": "gitdeck:send",
        "method": "pane.send_text",
        "params": { "pane_id": pane_id, "text": format!("\x1b[200~{clean}\x1b[201~") },
    });
    let mut stream = UnixStream::connect(&socket).context("cannot reach herdr socket")?;
    stream.set_read_timeout(Some(SOCKET_TIMEOUT))?;
    stream.set_write_timeout(Some(SOCKET_TIMEOUT))?;
    writeln!(stream, "{request}")?;
    let mut line = String::new();
    BufReader::new(stream)
        .read_line(&mut line)
        .context("herdr did not answer")?;
    let reply: Value = serde_json::from_str(&line).context("herdr reply is not JSON")?;
    if let Some(err) = reply.get("error") {
        bail!(
            "herdr refused: {}",
            err["code"].as_str().unwrap_or("unknown")
        );
    }
    Ok(())
}

/// Run `herdr <args>` for its effect only.
pub fn exec(args: &[&str]) -> Result<()> {
    let out = Command::new(bin())
        .args(args)
        .output()
        .context("failed to run herdr")?;
    if !out.status.success() {
        bail!(
            "herdr {}: {}",
            args.join(" "),
            String::from_utf8_lossy(&out.stderr).trim()
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    #[test]
    fn paste_keeps_text_and_drops_controls() {
        let evil = "fix\tthis\n\x1b[201~rm -rf\r\x03\u{9b}2J done";
        assert_eq!(super::sanitize_paste(evil), "fix\tthis\n[201~rm -rf2J done");
    }
}
