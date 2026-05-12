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

**Implicit init**: on the first `conductr begin` invocation in a working tree
that has no `.conductr`, the file is created automatically with `project_tag`
derived from the git remote `origin`:

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

**Cron / out-of-tree invocations must `cd` into a working tree** before calling
`conductr begin`:

```cron
# Correct: cd first so .conductr is found
*/30 * * * *  cd /home/user/projects/my-project && conductr begin --repo owner/my-project

# Wrong: no working tree → .conductr not found → init would fail or use wrong tag
*/30 * * * *  conductr begin --repo owner/my-project --tag my-project
```

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

### lockfile paths

```
~/.conductr/begin-foo.lock
~/.conductr/begin-bar.lock
```

### calendar event titles

```
[conductr/foo] orchestrate pass
[conductr/bar] orchestrate pass
```

### crontab entries

```cron
# foo — every 30 minutes
*/30 * * * *  cd ~/projects/foo && conductr begin --repo acme/foo

# bar — every 30 minutes, offset by 15
15,45 * * * *  cd ~/projects/bar && conductr begin --repo acme/bar
```

---

## Implicit `.conductr` init

When `conductr begin` runs in a directory that has no `.conductr`, it creates
one automatically:

1. Runs `git remote get-url origin` in the working directory.
2. Parses the repository name from the URL (handles both HTTPS and SSH forms).
3. Sanitises the name to `[a-z0-9-]` (uppercase → lowercase, invalid chars →
   `-`, leading/trailing `-` stripped).
4. Writes a minimal `.conductr` with `project_tag = "<derived-name>"`.

**Failure mode — no git remote**:

If `git remote get-url origin` fails (no remote configured), `conductr begin`
exits with:

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
| `conductr begin` — session name | `conductr-<tag>` | `conductr-<tag>-conductr` (agent = `conductr`) |
| `conductr diagnose/heal/free` — default filter | `"claude"` substring | `"conductr-"` prefix |
| lockfile | `~/.conductr/begin-<tag>.lock` | already correct |
| `.conductr` `project_tag` field | present but not validated | validated to `[a-z0-9-]` on read |

### `conductr begin` — session naming

**Before:** `conductr-<tag>` (e.g. `conductr-myproject`)

**After:** `conductr-<tag>-conductr` (e.g. `conductr-myproject-conductr`)

The `-conductr` suffix is the agent role.  Existing sessions created under the
old naming scheme will continue to work until they are next restarted, at which
point `begin` will create a session with the new name.

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
