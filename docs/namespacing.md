# Namespacing in conductr

This document codifies the five decisions made in issue #22 and the rules they
imply for every artefact `conductr` touches.

---

## The five decisions

### 1. Tag definition

A **project tag** is a free-form slug stored in `.conductr` under the key
`project_tag`.  It defaults to the repository name derived from the git remote
on first init.

Legal characters: `[a-z0-9-]`.  The `/` character is syntactically reserved for
the planned hierarchical form (see §3) but is not parsed today.

```toml
# .conductr
project_tag = "my-project"
```

### 2. What is namespaced

| Artefact | Namespaced? | Scope / key |
|----------|-------------|-------------|
| tmux sessions | **yes** | `conductr-<tag>-<agent>` |
| lockfiles | **yes** | `~/.conductr/begin-<tag>.lock` |
| calendar events | **yes** | title prefix `[conductr/<tag>]` |
| logs / telemetry | **yes** | tag label |
| beads queues | no | repo is the scope |
| GitHub labels | no | repo is the scope |
| `.conductr` contents | no | one file per repo |

**Tmux session convention**: `conductr-<tag>-<agent>` where `agent` ∈
`{conductr, agent<n>, qa<n>}`.

Examples for tag `"foo"`:

| Session | Role |
|---------|------|
| `conductr-foo-conductr` | orchestrator / cron entry point |
| `conductr-foo-agent1` | first parallel implementation agent |
| `conductr-foo-agent2` | second parallel implementation agent |
| `conductr-foo-qa1` | first QA / review agent |

### 3. One tag per repo

Strict one-tag-per-repo for now.  Every command reads the tag from the single
`.conductr` file at the repository root; there is no mechanism to override it.

**Planned trajectory**: hierarchical tags (`<repo>/<product>`) will allow one
physical repo to host multiple logical products.  The `/` separator is
syntactically reserved today — tags that contain `/` are rejected — but the
parser for hierarchical tags is deferred.  When it arrives, the tmux convention
becomes `conductr-<repo>-<product>-<agent>`.

### 4. `conductr` is a valid tag

The tag `"conductr"` is explicitly allowed.  The resulting tmux sessions for the
`conductr` repo itself are therefore named:

```
conductr-conductr-conductr   # orchestrator session
conductr-conductr-agent1     # first agent session
```

The apparent triple repetition (`conductr-conductr-conductr`) is intentional and
acceptable.

**Implicit init**: on the first `conductr begin` invocation (no-arg form) in a
working tree that has no `.conductr`, the file is created automatically with
`project_tag` derived from the git remote `origin`:

```toml
# .conductr — created automatically by `conductr begin`
project_tag = "my-project"
repo        = "owner/my-project"
```

See §5 and the [Implicit init](#implicit-conductr-init) section for failure
modes.

### 5. File only — no flag, no env var

The tag is resolved exclusively from the `.conductr` file.  There is no
`--tag` flag for overriding it and no `CONDUCTR_TAG` environment variable.

**Cron lines are installed by `conductr cadence sync`**, which reads `project_tag`
and `repo` from the `.conductr` file in the repo root. Run `conductr begin` once
to write the cadence defaults and install the entries:

```bash
cd /home/user/projects/my-project && conductr begin
```

The resulting cron lines look up `.conductr` via the cwd set in the cron entry:

---

## Worked examples: two coexisting projects

### Project layout

```
~/projects/
  foo/          ← project "foo"
    .conductr
    ...

  bar/          ← project "bar"
    .conductr
    ...
```

```toml
# ~/projects/foo/.conductr
project_tag = "foo"
repo        = "acme/foo"

# ~/projects/bar/.conductr
project_tag = "bar"
repo        = "acme/bar"
```

### tmux sessions

```
conductr-foo-conductr    ← foo's orchestrator
conductr-foo-agent1      ← foo's first implementation agent
conductr-foo-qa1         ← foo's QA agent

conductr-bar-conductr    ← bar's orchestrator
conductr-bar-agent1      ← bar's first implementation agent
```

Viewing the pod with `conductr diagnose` (default filter `conductr-`):

```
SESSION                HEALTH    IDLE   DETAIL
conductr-foo-conductr  idle        42s  waiting for first prompt
conductr-foo-agent1    working    215s  implementing issue #17
conductr-bar-conductr  idle        12s  waiting for first prompt
```

### calendar event titles

```
[conductr/foo] orchestrate pass
[conductr/bar] orchestrate pass
```

### crontab entries

Installed by `conductr cadence sync` (run `conductr begin` to generate and sync):

```cron
# conductr-cron: foo-orchestrate
*/30 * * * * bash -lc 'conductr orchestrate --repo acme/foo --once' >> ~/.local/share/conductr/orchestrate.log 2>&1

# conductr-cron: bar-orchestrate
15,45 * * * * bash -lc 'conductr orchestrate --repo acme/bar --once' >> ~/.local/share/conductr/orchestrate.log 2>&1
```

---

## Implicit `.conductr` init

When `conductr begin` (no-arg form) runs in a directory that has no `.conductr`,
it creates one automatically:

1. Runs `git remote get-url origin` in the working directory.
2. Parses the repository name from the URL (handles both HTTPS and SSH forms).
3. Sanitises the name to `[a-z0-9-]` (uppercase → lowercase, invalid chars →
   `-`, leading/trailing `-` stripped).
4. Writes a minimal `.conductr` with `project_tag = "<derived-name>"`.

**Failure mode — no git remote**:

If `git remote get-url origin` fails (no remote configured), `conductr begin`
(no-arg form) exits with:

```
error: no git remote 'origin' found; cannot derive project tag automatically.
       Create a .conductr file manually with `project_tag = "<name>"`
```

In this case, create `.conductr` by hand before re-running the command.

---

## Codebase audit

This table maps every place a project identity is inferred implicitly to the
explicit rule it now follows.

| Location | Previous behaviour | Rule (this doc) |
|----------|--------------------|-----------------|
| cron entry-point | `conductr begin --tag <tag> --repo <slug>` | `conductr orchestrate --repo <slug> --once` (installed by `cadence sync`) |
| `conductr diagnose/heal/free` — default filter | `"claude"` substring | `"conductr-"` prefix |
| `.conductr` `project_tag` field | present but not validated | validated to `[a-z0-9-]` on read |

### `conductr begin` — new role

`begin` is now a **cadence configurator**, not a cron entry-point.  It writes
cadence entries into `.conductr [cadence]` and delegates to `cadence sync` to
install the actual cron lines. The cron lines invoke `conductr <skill>` directly
(e.g. `conductr orchestrate --repo <slug> --once`).  Session management is the
responsibility of each Claude-required command (like `architect` and `idle`).

### Pod filter default

**Before:** sessions whose name contains the substring `"claude"`

**After:** sessions whose name starts with the prefix `"conductr-"`

Commands that accept `--pattern` (`diagnose`, `free`, `heal`, `save-state`) are
unaffected — the filter is applied to the session name string; only the default
changes.  Pass `--all` to inspect every tmux session regardless of name.

---

## Non-goals (from issue #22)

- **Renaming sessions in flight.** The new naming applies only to sessions
  created after this change.  Existing sessions migrate when they next restart.
- **Hierarchical tags.** Deferred.  The `/` separator is reserved; the parser
  is not implemented.
- **`--tag` flag or `CONDUCTR_TAG` env var.** Explicitly declined.  The file
  is the single source of truth.
