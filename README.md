# herdr-gitdeck

[![ci](https://github.com/pgen0x/herdr-gitdeck/actions/workflows/ci.yml/badge.svg)](https://github.com/pgen0x/herdr-gitdeck/actions/workflows/ci.yml)
[![release](https://img.shields.io/github/v/release/pgen0x/herdr-gitdeck)](https://github.com/pgen0x/herdr-gitdeck/releases)
[![license: MIT](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE)

A [herdr](https://herdr.dev) plugin to review what your coding agents changed, **across every
git repo in a folder**, without leaving the terminal. Browse diffs, stage or discard single
hunks, commit, and send line comments straight back to the agent that wrote the code.

```text
╭ repos 4 · 2 dirty ───────────────────────╮┏▸ src/auth.py changes · unified ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━┓
│● api-server ~1 ?1  main                  │┃@@ -2,11 +2,12 @@ import jwt                                    ┃
│● web-app +1  main                        │┃    2    2                                                      ┃
│● docs-site  main                         │┃    3    3  COOKIE_NAME = "session"                             ┃
│● infra  main                             │┃    4    4  MAX_AGE = 3600                                      ┃
│                                          │┃         5 +COOKIE_DOMAIN = ".example.com"                      ┃
│                                          │┃    5    6                                                      ┃
│                                          │┃    6    7                                                      ┃
│                                          │┃    7    8  def issue_token(user_id: str, secret: str) -> str:  ┃
╰──────────────────────────────────────────╯┃    8    9      """Sign a session token for the given user."""  ┃
╭ files api-server ────────────────────────╮┃    9      -    payload = {"sub": user_id}                      ┃
│CHANGES 1                                 │┃        10 +    payload = {"sub": user_id, "iat": now()}        ┃
│ M auth.py src/                           │┃   10   11      return jwt.encode(payload, secret, algorithm="HS┃
│UNTRACKED 1                               │┃   11   12                                                      ┃
│ ? requirements.txt                       │┃   12   13                                                      ┃
│                                          │┃@@ -15,4 +16,6 @@ def cookie_options() -> dict:                 ┃
│                                          │┃   15   16          "httponly": True,                           ┃
│                                          │┃   16   17          "secure": True,                             ┃
│                                          │┃   17   18          "max_age": MAX_AGE,                         ┃
│                                          │┃        19 +        "domain": COOKIE_DOMAIN,                    ┃
│                                          │┃        20 +        "samesite": "lax",                          ┃
│                                          │┃   18   21      }                                               ┃
╰──────────────────────────────────────────╯┗━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━┛
space stage hunk · x discard hunk · n/p hunk · c comment · v split                                     ? help
```

*(The real pane is in color: syntax-highlighted code on green/red tinted rows.)*

## Why

When several agents work in parallel, the changes land in many repos at once. Single-repo
review tools refuse to open in a folder that is not itself a repository, such as
`~/projects`. gitdeck opens anywhere. Inside a repo it shows that repo. In any other folder it
finds every repo below it and lists the ones with changes first.

## Features

- **Multi-repo dashboard**: every repo under the folder, with staged (`+`), changed (`~`) and
  untracked (`?`) counts, ahead/behind (`↑` `↓`) and branch. Dirty repos sort first, and `a`
  hides the clean ones. Status refreshes in the background every 3 seconds.
- **Diff viewer**: unified or side-by-side split view (`v`), syntax highlighting for the
  languages [bat](https://github.com/sharkdp/bat) supports, and hunk navigation.
- **Stage, unstage, discard**: whole files or single hunks, in the index or the working
  tree. Destructive actions ask first and name the exact file or hunk.
- **Commit** staged changes without leaving the pane.
- **Review comments to your agent**: comment on a line or a file, then send all comments for
  the repo to the agent working in it as one paste. Nothing is submitted for you; you read the
  paste and press Enter in the agent pane.
- **Responsive layout**: three panes side by side when wide, stacked when narrow, and the
  focused panel gets the space.
- **Safe with agents running**: read-only git calls take no `index.lock`, so a background
  rescan never collides with an agent committing.

## Requirements

- herdr **0.9.3** or newer
- `git` on `PATH`
- Linux or macOS (x86_64 or arm64)
- Rust, only to build from source. Release binaries are downloaded automatically.

## Install

```sh
herdr plugin install pgen0x/herdr-gitdeck
```

herdr shows a trust preview first. The build step downloads the release binary for your
platform and checks its SHA-256. If no release exists for your platform it builds from source
with `cargo`.

Then bind a key in `~/.config/herdr/config.toml` and reload the config with
`herdr server reload-config`:

```toml
[[keys.command]]
key = "prefix+shift+g"
type = "plugin_action"
command = "pgen0x.gitdeck.toggle"
description = "gitdeck: multi-repo review"
```

`prefix+g` is already herdr's `goto`, so pick a free chord. You can also open it without a key:
`herdr plugin action invoke toggle --plugin pgen0x.gitdeck`.

## Usage

Press your key in any pane. gitdeck opens as a split to the right, in that pane's current
directory:

| Started in | Shows |
| --- | --- |
| a git repo (or any folder inside one) | that repo |
| any other folder | every repo up to 3 levels below it (skips hidden folders, `node_modules`, `target`, `vendor`, `dist`, `build`, `venv`) |

The same key closes the pane again.

### Keys

| Key | Where | Action |
| --- | --- | --- |
| `Tab` `Enter` `l` `→` | anywhere | next panel (repos → files → diff) |
| `Shift-Tab` `Esc` `h` `←` | anywhere | previous panel |
| `j` `k` `↓` `↑` | lists, diff | move |
| `g` `G` `Home` `End` | lists, diff | first / last |
| `Ctrl-d` `Ctrl-u` `PgDn` `PgUp` | diff | page down / up |
| `n` `p` | diff | next / previous hunk |
| `Space` | files | stage or unstage the file |
| `Space` | diff | stage or unstage the hunk under the cursor |
| `x` | files / diff | discard the file / hunk (asks first) |
| `v` | anywhere | toggle unified and split diff |
| `a` | anywhere | show only repos with changes |
| `r` | anywhere | rescan every repo now |
| `C` | anywhere | commit the staged changes of the selected repo |
| `c` | files / diff | comment on the file / the line under the cursor |
| `S` | anywhere | send this repo's comments to the agent |
| `X` | anywhere | drop every queued comment |
| `?` | anywhere | key help |
| `q` `Ctrl-c` | anywhere | quit |

In the commit and comment prompts, `Enter` submits, `Alt-Enter` adds a new line and `Esc`
cancels.

### Sending review comments

1. Move to a line in the diff (or a file in the list) and press `c`. Type the comment and
   press `Enter`. Lines with comments get a `●` marker. Repeat as needed.
2. Press `S`. gitdeck picks the agent in the **same herdr workspace** whose working directory
   is inside the repo, or else the first agent in the workspace. It pastes the review into
   that agent's input and focuses its pane. You submit it.

The agent receives plain text:

```text
Review comments on /home/me/projects/api-server:

src/auth.py:10
  > payload = {"sub": user_id, "iat": now()}
  `now` is not imported here

src/auth.py (deleted line, was line 9)
  > payload = {"sub": user_id}
  keep the old shape for v1 tokens
```

The paste is sent as one bracketed paste over herdr's socket API. Control characters are
removed first, because snippets come from repository content and could otherwise act as key
presses. An agent that is waiting on a permission prompt is refused rather than pasted into.
If sending fails, your comments are kept for another try.

## Troubleshooting

**The pane draws at half width on first open.** In herdr 0.9.3, a plugin pane opened with
focus keeps its pre-layout terminal size until the focus changes. gitdeck works around this by
opening unfocused and then moving focus itself. If you still see it, report your herdr version.

**Nothing happens on the key.** Check the action log:

```sh
herdr plugin log list --plugin pgen0x.gitdeck
```

**It feels slow.** Start a pane with timing logs and send the file:

```sh
herdr plugin pane open --plugin pgen0x.gitdeck --entrypoint pane --placement split \
  --direction right --target-pane "$HERDR_PANE_ID" --cwd ~/projects --env GITDECK_LOG=/tmp/gitdeck.log
```

## Limitations

- Linux and macOS only (Windows is not supported yet).
- Hunks are the smallest unit. Staging single lines is not supported.
- Diffs over 4,000 lines are shown without syntax highlighting.
- Highlighting starts fresh at each hunk, so a hunk that opens inside a block comment or a
  multi-line string can be colored wrong until it closes.
- Hunk operations on files that are not valid UTF-8 fail with git's error. Nothing is changed.
- Repos are discovered when the pane opens. Reopen it to pick up a newly cloned repo.

## Development

```sh
git clone https://github.com/pgen0x/herdr-gitdeck && cd herdr-gitdeck
GITDECK_FROM_SOURCE=1 bash herdr/install.sh   # build into bin/
herdr plugin link "$PWD"                      # use this checkout as the plugin
cargo test
```

After a change, rebuild with `bash herdr/install.sh` and reopen the pane. `bin/` is replaced in
place, so a running pane is not affected.

| Module | Role |
| --- | --- |
| `src/main.rs` | entry point, event loop, `--action` dispatch |
| `src/actions.rs` | plugin actions: `toggle`, `open`, `close` |
| `src/repos.rs` | repo discovery |
| `src/scan.rs` | background `git status` thread |
| `src/git.rs` | git CLI wrapper: status, diff, stage, discard, commit |
| `src/diff.rs` | unified diff parser, single-hunk patches, split-view rows |
| `src/highlight.rs` | syntax highlighting (syntect and the bat syntax set) |
| `src/herdr.rs` | herdr CLI and socket: agents, panes, paste |
| `src/app.rs` | state and key handling |
| `src/ui.rs` | rendering (ratatui) |

See [CONTRIBUTING.md](CONTRIBUTING.md) for the release process.

## Credits

Inspired by [herdr-reviewr](https://github.com/persiyanov/herdr-reviewr), which pioneered
sending review comments from a diff pane to a herdr agent.

## License

[MIT](LICENSE)
