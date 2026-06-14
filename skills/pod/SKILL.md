---
name: pod
description: Diagnose, heal, and save-state across the local Claude Code pod (other tmux sessions on this host). Use when the user asks to check / restart / snapshot other Claude threads, e.g. "are the other threads ok", "restart everything to pick up new skills", "save state across the pod before I reboot".
cli: conductr pod diagnose|heal|save-state|free [...]
---

# pod

This skill drives the `conductr` CLI to manage the *local* Claude Code pod —
every tmux session on this host with a `claude` agent in it. It is the
in-Claude counterpart to `conductr pod diagnose|heal|save-state` and adds the
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
  name prefix; default is `conductr-` (covers all `conductr-<tag>-<agent>`
  sessions).  Sessions created before the namespacing convention can be
  included with `--all`.
- For `conductr pod save-state --tracker beads` (default): `br` (beads) installed and a
  beads database initialised in `~/.beads` or the current directory.
- For `conductr pod save-state --tracker notion`: `NOTION_API_KEY` set and
  `--notion-database <id>` supplied (or `CONDUCTR_NOTION_DATABASE` env var).

If any of these are missing, *say so to the user and stop* — do not silently
fall back to ad-hoc tmux scripting.

## Subcommands

### diagnose

```
conductr pod diagnose            # human-readable table
conductr pod diagnose --json     # machine-readable
```

Run it, then summarise in a short table or bullet list. The `idle_seconds`
column is wall-clock time since tmux saw any activity in the pane — it does
*not* mean Claude has been idle for that long, just that nothing has been
typed or rendered. Don't over-interpret it.

### heal

```
conductr pod heal                            # provision (all active) + restart crashed
conductr pod heal --dry-run                  # preview without making changes
conductr pod heal --repo Luan-vP/foo         # scope both passes to one project
conductr pod heal --no-provision             # skip provisioning; restart-only (old behaviour)
conductr pod heal --command 'claude --continue'   # custom relaunch/launch command
```

`heal` runs two passes:

1. **Provision pass** — delegates to `conductr setup spawn` for every in-scope
   project in `~/.conductr`: ensures the clone, `.conductr`, cron markers, and
   `conductr-<tag>` tmux session all exist, booting Claude into any session it
   creates. Idempotent, so this is what brings *dropped* sessions back. Skipped
   with `--no-provision`, or when there's no registry (heal then degrades to the
   old "just restart crashed sessions" behaviour).
2. **Restart pass** — anything classified `crashed` gets `--command` sent to it.
   Idle / working sessions are left alone. Sessions the provision pass just
   created are excluded here (they were already launched).

`--command` feeds both passes and defaults to
`claude --remote-control --permission-mode auto`, so restored sessions come up
with Remote Control enabled in `auto` permission mode — immediately driveable by
orchestrate.

The in-scope set is determined by `--repo`:

- **without `--repo`**: every `status = "active"` entry in `~/.conductr`.
- **with `--repo owner/name`**: just that one active entry. Errors if no active
  project has the slug. The restart pass is exact-name matched on
  `conductr-<tag>`, so sibling sessions (`conductr-foo-dashboard-*`) aren't
  collateral damage.

`--json` emits `{ "provision": ProjectReport[], "heal": HealOutcome[] }` — the
provision reports share the `setup spawn` step schema (`clone` · `dot-conductr`
· `cadence` · `session` · `launch`).

`heal` only ever *restarts* crashed sessions; it won't bounce a healthy one. To
restart healthy sessions too (e.g. to pick up a new skill), use `save-state`.

### save-state

```
conductr pod save-state                                        # capture work, write beads issues, restart
conductr pod save-state --tracker beads                       # explicit beads (default)
conductr pod save-state --tracker notion --notion-database <id>  # write to Notion database
conductr pod save-state --dry-run                             # plan only, no writes, no restarts
conductr pod save-state --no-restart                          # capture only, leave panes running
```

`--tracker notion` also accepts the database ID via `CONDUCTR_NOTION_DATABASE` (env var).

Output is a JSON manifest with one entry per pod session:

```jsonc
{
  "session": "claude-thread2",
  "health": "idle",
  "cwd": "/home/dev/developer/foo",
  "last_message": "try out the conductor",
  "tail": ["...", "..."],
  "tracker": "beads",                      // "beads" or "notion"
  "tracker_id": "br-thread-recovery-...", // null if no recoverable work or --dry-run
  "restart": "restarted:exit-then-relaunch"
}
```

**Your job around `save-state`:** behaviour depends on which tracker was used.

#### When `tracker == "beads"` (default)

The binary wrote the recovery issue to beads. If the user has Notion tickets
that mirror their beads work, *you* update those tickets — the binary didn't
do it. The contract is:

1. Run `conductr pod save-state --json` (or `--dry-run` first if the user wants a
   preview).
2. Parse the manifest.
3. For each entry where `tracker_id` is non-null and `tracker == "beads"`:
   - If a Notion mirror page/database is configured (ask the user once per
     session if not), upsert a Notion record with the same title/body and a
     link back to the tracker id. Use `mcp__claude_ai_Notion__notion-search`
     to find an existing page by `tracker_id`, then `notion-update-page` or
     `notion-create-pages` accordingly.
   - If you have no Notion access in this session, *say so* and skip — do
     not pretend it succeeded.
4. Report a final table: session → tracker id → notion page url → restart
   action.

#### When `tracker == "notion"`

The binary already wrote the recovery issue directly into Notion. You do **not**
need to mirror anything. Just report the results from the manifest.

1. Run `conductr pod save-state --tracker notion --notion-database <id> --json`.
2. Parse the manifest.
3. Report a final table: session → tracker id → restart action.

### free

```
conductr pod free                  # print tmux attach command for an idle session
conductr pod free --json           # machine-readable
conductr pod free --include-attached  # also consider attached sessions
```

Exits non-zero if no idle session is found.

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
  state. Use `conductr pod save-state` (which sends `/exit`) or `conductr pod heal`.
- **Don't** fabricate Notion updates. If you don't have Notion tools wired up
  in this session, say so.
- **Don't** run `conductr pod save-state` on `--all` (every tmux session) without
  confirming. Non-claude sessions probably shouldn't be sent `/exit`.
- **Don't** parse the human-readable output. Always pass `--json` when you
  intend to act on the result.
