---
name: conductr-pod
description: Diagnose, heal, and save-state across the local Claude Code pod (other tmux sessions on this host). Use when the user asks to check / restart / snapshot other Claude threads, e.g. "are the other threads ok", "restart everything to pick up new skills", "save state across the pod before I reboot".
---

# conductr-pod

This skill drives the `conductr` CLI to manage the *local* Claude Code pod —
every tmux session on this host with a `claude` agent in it. It is the
in-Claude counterpart to `conductr diagnose|heal|save-state` and adds the
external-system updates (Notion etc.) that the binary deliberately omits.

## When to invoke

- *"Check the other threads"* → `diagnose`
- *"Restart anything that crashed"* → `heal`
- *"Save state, restart everything (e.g. to pick up new skills)"* → `save-state`

If the user mentions a remote host or a fleet of instances, this is the wrong
skill — that's `conductr instance` territory (currently stubbed). This skill
only handles the local-tmux pod.

## Prerequisites

- `conductr` on `$PATH` (`cargo install --path crates/conductr` from the repo).
- `tmux` running with the user's pod sessions. Pod sessions are matched by
  name substring; default is `claude`.
- For `save-state`: `br` (beads) installed and a beads database initialised in
  `~/.beads` or the current directory.

If any of these are missing, *say so to the user and stop* — do not silently
fall back to ad-hoc tmux scripting.

## Subcommands

### diagnose

```
conductr diagnose            # human-readable table
conductr diagnose --json     # machine-readable
```

Run it, then summarise in a short table or bullet list. The `idle_seconds`
column is wall-clock time since tmux saw any activity in the pane — it does
*not* mean Claude has been idle for that long, just that nothing has been
typed or rendered. Don't over-interpret it.

### heal

```
conductr heal                # restart anything classified `crashed`
conductr heal --dry-run      # preview without sending keys
conductr heal --command 'claude --continue'   # custom relaunch command
```

`heal` only acts on `crashed` sessions. Idle / working sessions are left
alone. If the user wants to *also* restart healthy sessions (e.g. to pick up
a new skill), use `save-state` instead.

### save-state

```
conductr save-state                 # capture work, write beads issues, restart
conductr save-state --dry-run       # plan only, no writes, no restarts
conductr save-state --no-restart    # capture only, leave panes running
```

Output is a JSON manifest with one entry per pod session:

```jsonc
{
  "session": "claude-thread2",
  "health": "idle",
  "cwd": "/home/dev/developer/foo",
  "last_message": "try out the conductor",
  "tail": ["...", "..."],
  "beads_id": "br-thread-recovery-...",   // null if no recoverable work
  "restart": "restarted:exit-then-relaunch"
}
```

**Your job around `save-state`:** the binary deliberately stops at beads. If
the user has Notion tickets that mirror their beads work, *you* update those
tickets too — the binary doesn't ship a Notion connector for this flow. The
contract is:

1. Run `conductr save-state --json` (or `--dry-run` first if the user wants a
   preview).
2. Parse the manifest.
3. For each entry where `beads_id` is non-null:
   - If a Notion mirror page/database is configured (ask the user once per
     session if not), upsert a Notion record with the same title/body and a
     link back to the beads id. Use `mcp__claude_ai_Notion__notion-search`
     to find an existing page by `beads_id`, then `notion-update-page` or
     `notion-create-pages` accordingly.
   - If you have no Notion access in this session, *say so* and skip — do
     not pretend it succeeded.
4. Report a final table: session → beads id → notion page url → restart
   action.

### Restart semantics

- `idle` sessions are restarted with `/exit` followed by the launch command.
  This loses the in-memory conversation but preserves the cwd and the
  user's Claude config (so newly added skills/plugins are picked up).
- `working` sessions are *skipped* — interrupting a turn would lose state.
  Tell the user, and offer to retry once they finish.
- `crashed` sessions are restarted with the launch command directly.
- `unknown` sessions are skipped with a warning.

## Anti-patterns

- **Don't** `tmux kill-session` to "restart" a thread. You lose the cwd, the
  scrollback that contains the user's last message, and any untracked shell
  state. Use `save-state` (which sends `/exit`) or `heal`.
- **Don't** fabricate Notion updates. If you don't have Notion tools wired up
  in this session, say so.
- **Don't** run `save-state` on `--all` (every tmux session) without
  confirming. Non-claude sessions probably shouldn't be sent `/exit`.
- **Don't** parse the human-readable output. Always pass `--json` when you
  intend to act on the result.
