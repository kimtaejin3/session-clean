# sclean

[![CI](https://github.com/kimtaejin3/session-clean/actions/workflows/ci.yml/badge.svg)](https://github.com/kimtaejin3/session-clean/actions/workflows/ci.yml)
[![npm](https://img.shields.io/npm/v/session-clean)](https://www.npmjs.com/package/session-clean)
[![license](https://img.shields.io/badge/license-MIT-blue)](LICENSE)

A local terminal UI for reviewing the sessions your coding agents leave behind, and clearing them out safely — with a reason shown for every suggestion.

```text
┌ Projects 2/5 ──────────────┐┌ Codex · shop-api — 2 sessions ─────────────── Trash: 0 ┐
│  ── Claude Code            ││  [x] ★ 92d ago  Fix login redirect   last active 92 d… │
│  blog                      ││  [ ] ★ 45d ago  Build error question 1 user message,… │
│  shop-api          2 sugg  ││                                                        │
│▶ ── Codex                  ││                                                        │
│  shop-api          2 sugg  ││                                                        │
│  ── Gemini CLI (unverified)││                                                        │
│  a1b2c3d4          unknown ││                                                        │
│  ── Continue               ││                                                        │
│  shop-api          1 sugg  ││                                                        │
└────────────────────────────┘└────────────────────────────────────────────────────────┘
 9 sessions · 5 suggested · 1 selected (1 KB)
 ↑↓ sessions  ← projects  Space select  A suggested  D clean  T trash  F rules  ? help  Q quit
```

State is readable from symbols alone, never from color: `[x]` selected, `[ ]` not selected, `[-]` cannot be cleaned, `★` suggested, `!` unparseable, `▶` running.

## Install

```sh
npx session-clean
```

That is enough to try it once. To keep it around, install it globally:

```sh
npm install -g session-clean
sclean
```

The package is named `session-clean`; the command is **`sclean`**.

To build from source you need Rust 1.85 or newer (edition 2024):

```sh
cargo install --path .
```

## Supported agents

sclean only handles agents that store **one session per file**. Its safety model is "move the file to a trash folder, and move it back on restore" — that does not translate to agents which keep sessions as rows in a database (Cursor, OpenCode, Goose, Crush, Amp), so those are out of scope.

| Agent | Location | Format confirmed |
|---|---|---|
| Claude Code | `~/.claude/projects/<cwd>/<uuid>.jsonl` | against real data |
| Codex | `~/.codex/sessions/YYYY/MM/DD/rollout-*.jsonl` | against real data |
| Continue | `~/.continue/sessions/<uuid>.json` | against real data |
| Gemini CLI | `~/.gemini/tmp/<hash>/chats/*.json` | from documentation |
| Copilot CLI | `~/.copilot/session-state/` | from documentation |

The two agents whose format was only taken from documentation are labelled `(미검증)` — *unverified* — in the interface. If their real format differs, their sessions are marked unparseable and **cleanup is refused**: sclean does nothing rather than delete the wrong thing.

Agents you do not have installed are skipped silently. Every target path is checked against its own agent's data directory, so cleaning one agent can never touch another's files.

## Platforms

| Platform | Status |
|---|---|
| macOS arm64 / x64 | supported (primary test environment is Apple Silicon) |
| Linux x64 / arm64 | supported |
| Windows | not supported natively — **use WSL** |

Windows builds fail because sclean uses Unix-only filesystem APIs (symlinks, process signals). If you run your coding agents inside WSL, their data lives in WSL too and the Linux build works as-is.

Settings and the trash folder live in `~/Library/Application Support/sclean` on macOS and `~/.local/share/sclean` on Linux.

## Running it

```sh
sclean
```

There are no flags. Selecting sessions, changing the criteria, moving to trash, deleting permanently, and restoring all happen inside the TUI.

## Keys

The left pane lists projects grouped by agent; the right pane shows **only the sessions of the selected project**. `→` moves into the session list, `←` goes back. On a narrow terminal only the pane you are working in is shown, at full width.

| Key | Action |
|---|---|
| `↑` `↓` | Move within the current pane |
| `→` | Open the selected project's sessions |
| `←` | Back to the project list |
| `Space` | Select / deselect (on the project list, the whole project) |
| `A` | Select / deselect every suggested session |
| `D` | Clean up the selection |
| `T` | Trash |
| `F` | Suggestion criteria |
| `?` | Help |
| `Q` | Quit |

In the confirmation screen, `←` `→` choose between **move to trash** and **delete permanently**. Permanent deletion requires typing `DELETE`.

In the trash screen: `R` restores, `X` deletes permanently, `Enter` shows details.

## How sessions are suggested

A session is suggested when it matches at least one rule, and the reason is always shown next to it. Each rule can be toggled from the `F` screen.

| Rule | Condition | Default |
|---|---|---|
| R1 Old | Last activity older than the threshold | on / 30 days |
| R2 Missing project | The session's `cwd` no longer exists | on |
| R3 Short session | At most one user message and no tool calls | on |
| R4 Finished subagent | A subagent session with no recent activity | on |
| R5 Orphaned data | Related data left behind with no transcript | on |

A session is **never suggested, and sometimes refused outright**, when:

- its format cannot be parsed (refused)
- the owning session of its related data cannot be established (refused)
- it is currently running (refused)
- its size or mtime changed since the scan (refused)
- its project path could not be established (not suggested)
- it was active in the last five minutes (not suggested)

Suggestions never select anything for you. You select sessions yourself, or press `A`.

## What it reads and writes

Only paths that actually exist are read; a missing one is not an error.

**Claude Code**

```text
~/.claude/projects/<project>/<session>.jsonl   transcript
~/.claude/projects/<project>/<session>/        subagent transcripts
~/.claude/tasks/  teams/                       keyed by the first 8 characters of the
                                               session id — claimed only when unambiguous
~/.claude/session-env/  file-history/  todos/  debug/
~/.claude/sessions/                            run locks (read only)
~/.claude/history.jsonl                        shared record (only lines with a known owner)
```

**Other agents**

```text
~/.codex/sessions/YYYY/MM/DD/rollout-*.jsonl   transcript (cwd is on the first line)
~/.codex/archived_sessions/                    archived transcripts
~/.continue/sessions/<uuid>.json               transcript (has title and workspaceDirectory)
~/.gemini/tmp/<project_hash>/chats/*.json      transcript (project is a hash only)
~/.copilot/session-state/                      transcript
```

sclean writes to exactly one place:

```text
~/Library/Application Support/sclean/   (Linux: ~/.local/share/sclean/)
├── config.json      suggestion criteria
├── sclean.log       local log — prompt bodies are never written to it
└── trash/<op-id>/   manifest.json plus the moved files
```

Your project source files are **never read or written**, only checked for existence.

## Safety

- Every target file is re-checked immediately before the operation; sessions that changed since the scan are excluded.
- Every target path is verified to sit inside its own agent's data directory. If a single one falls outside, the whole operation is abandoned. Symlinks are never followed.
- `manifest.json` is written **before** any file moves, so an interrupted run can be recovered on the next start.
- Shared files such as `history.jsonl` are backed up, rewritten to a temporary file, and swapped in atomically.
- Restoring never overwrites: if something already occupies the original path, the item is reported as a conflict and left in the trash.
- Cleanup and restore are idempotent — running them twice does not double-apply.
- No network access, no telemetry, no account. Everything stays on your machine.

## Development

```sh
cargo test                       # 212 tests
cargo clippy --all-targets -- -D warnings
cargo fmt --check
cargo test --release --test perf_test -- --nocapture   # scan time for 2,000 sessions
```

Tests never touch your real agent directories; they build fixtures in a temporary directory. To point a real run somewhere else:

```sh
SCLEAN_HOME=/tmp/fake-home SCLEAN_DATA_DIR=/tmp/fake-sclean sclean
```

To eyeball the layout, print the rendered screens:

```sh
cargo test --test render_test visual_dump -- --ignored --nocapture
```

### Adding an agent

Implement `agents::Agent` — the adapter only needs to find session files, parse them, and list related files. Rule evaluation, the cleanup transaction, the trash and restore are shared. Register it in `agents::registry()` and add its directory to `paths::AGENT_DIRS`.

### Releasing

npm carries prebuilt native binaries, one package per platform, and a thin JS launcher picks the one matching `os`/`cpu`.

```text
session-clean                      what users install (bin/sclean.js launcher)
├── session-clean-darwin-arm64     optionalDependencies — npm installs only the match
├── session-clean-darwin-x64
├── session-clean-linux-x64
└── session-clean-linux-arm64
```

`darwin-x64` is cross-compiled on an arm64 runner, because the Intel macOS queue is slow; only the build is verified for that target, not the tests.

Pushing a tag builds all four platforms and publishes to npm and GitHub Releases:

```sh
git tag v0.2.0 && git push origin v0.2.0
```

If the CI token is blocked by npm's 2FA policy, publish from your machine using the artifacts CI already built:

```sh
npm login
./scripts/publish-local.sh <run-id>
```

## Roadmap

Considered only if people keep asking:

- Session search
- Agents that store sessions in SQLite (Cursor, OpenCode, Goose)
- Native Windows support
- Homebrew and auto-update
- Presets for the suggestion rules

## License

MIT — [LICENSE](LICENSE)
