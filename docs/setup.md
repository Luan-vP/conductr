# Conductr Setup Guide

`conductr setup` checks whether a repository is ready for `conductr orchestrate` and
related automation. It walks a six-level maturity checklist, reports the current level,
and can apply the fixable items automatically.

## Quick start

```sh
# See where your repo stands
conductr setup status

# Fix what can be fixed automatically (asks before each change)
conductr setup wizard

# Fix everything without prompting
conductr setup wizard --non-interactive

# Preview all fixes with no side effects
conductr setup wizard --dry-run

# Open the Claude GitHub App install page
conductr setup install-claude-app
```

---

## Maturity levels

| Level | Name          | What it means                                              |
|-------|---------------|------------------------------------------------------------|
| L0    | Bootstrap     | Starting point — no checks required                       |
| L1    | Tested        | The workspace builds and tests run on CI                  |
| L2    | GitFlow       | A `dev`/`develop` branch exists; PRs target it            |
| L3    | Architected   | Architecture docs, CONTRIBUTING, and CODEOWNERS are in place |
| L4    | Skilled       | Claude skills and agents are configured                   |
| L5    | Orchestrated  | The Claude GitHub App is installed and the workflow runs  |

Each level is cumulative: reaching L3 means L1 and L2 also pass.

---

## Checks

### L1 Tested

#### `ci-workflow` — .github/workflows/*.yml runs tests on push
**Fixable:** yes (`add_ci_workflow`)

At least one YAML file must exist in `.github/workflows/`. The generated workflow
runs `cargo test --workspace` on every push.

#### `gitignore-target` — .gitignore covers target/
**Fixable:** yes (`add_gitignore_target`)

The `.gitignore` file must contain `target/` or `target` on its own line.

---

### L2 GitFlow

#### `dev-branch` — dev (or develop) branch exists
**Fixable:** yes (`init_git_flow`)

A local branch named `dev` or `develop` must exist. The fix runs
`git checkout -b dev` from the repo root.

#### `default-base-dev` — default PR base is dev/develop
**Fixable:** no (requires GitHub repository settings)

This check passes when the `dev`/`develop` branch exists (used as a
proxy). To set the true default branch, go to **Settings → Branches** on
GitHub and change the default branch to `dev`.

---

### L3 Architected

#### `claude-base-md` — .claude/base.md exists
**Fixable:** no

`.claude/base.md` is the architecture reference that Claude reads before
every task. Create it by describing the hexagonal layout of your codebase.
See this repository's `.claude/base.md` for a template.

#### `contributing-md` — CONTRIBUTING.md mentions architecture conventions
**Fixable:** no

`CONTRIBUTING.md` (or `.rst`/`.txt`) must contain at least one of the words
`architect`, `convention`, or `structure` (case-insensitive).

#### `codeowners` — CODEOWNERS present
**Fixable:** yes (`add_codeowners`)

A `CODEOWNERS` file must exist at the repo root, in `.github/`, or in
`docs/`. The fix writes a minimal template to `CODEOWNERS`.

---

### L4 Skilled

#### `skill-md` — at least one skills/<name>/SKILL.md exists
**Fixable:** no

The `skills/` directory must contain at least one subdirectory with a
`SKILL.md` file. Skills are markdown documents that describe how Claude
should use a particular CLI subcommand.

#### `claude-agents` — .claude/agents/ directory exists
**Fixable:** no

`.claude/agents/` must be a directory. Add at least one agent definition
file to this directory.

---

### L5 Orchestrated

#### `claude-app` — Claude GitHub App installed (manual step)
**Fixable:** yes (opens install URL; never auto-installs)

The Claude GitHub App must be installed on the repository. This cannot be
verified locally, so the check always reports as unverified. Run
`conductr setup install-claude-app` to open the install page.

After installing:
1. Grant access to this repository.
2. Re-run `conductr setup status` — the workflow check (below) will tell
   you whether end-to-end wiring is in place.

#### `claude-workflow` — .github/workflows/claude.yml present
**Fixable:** yes (`add_claude_workflow`)

`.github/workflows/claude.yml` must exist. The fix writes a workflow that
triggers on `@claude` mentions in issues and pull requests and calls the
`anthropics/claude-code-action`.

---

## CLI reference

### `conductr setup status [--repo <path>]`

Runs all checks and prints a pass/fail table with the achieved level.
Makes no changes to the repository.

```
Repo:  /home/user/myproject
Level: L3 Architected

  ✓ [L1 Tested] .github/workflows/*.yml runs tests on push
  ✓ [L1 Tested] .gitignore covers target/
  ✓ [L2 GitFlow] dev (or develop) branch exists
  ✓ [L2 GitFlow] default PR base is dev/develop
  ✓ [L3 Architected] .claude/base.md exists
  ✓ [L3 Architected] CONTRIBUTING.md mentions architecture conventions
  ✓ [L3 Architected] CODEOWNERS present
  ✗ [L4 Skilled] at least one skills/<name>/SKILL.md exists
    → no skills/<name>/SKILL.md found
  ✗ [L4 Skilled] .claude/agents/ directory exists
    → .claude/agents/ directory not found
  ✗ [L5 Orchestrated] Claude GitHub App installed (manual step)
    → cannot verify automatically — install the Claude GitHub App manually
  ✗ [L5 Orchestrated] .github/workflows/claude.yml present
    → .github/workflows/claude.yml not found
```

### `conductr setup wizard [--repo <path>] [--non-interactive] [--dry-run]`

Runs the same checks, then offers to apply fixable ones.

| Flag                | Behaviour                                           |
|---------------------|-----------------------------------------------------|
| (none)              | Interactive — prompts `[y/N]` before each fix       |
| `--non-interactive` | Applies all fixable checks without prompting        |
| `--dry-run`         | Prints what would be done; touches nothing          |

Non-fixable checks (e.g. `claude-base-md`, `skill-md`) are listed but
skipped automatically. L5 stops with an "open this URL" message for the
Claude App — the wizard never installs it.

### `conductr setup install-claude-app [--repo <path>]`

Prints the Claude GitHub App install URL and follow-up instructions.

---

## Bringing a repo from L0 to L4 in one shot

```sh
git init my-new-repo && cd my-new-repo
conductr setup wizard --non-interactive
# → writes .github/workflows/ci.yml
# → appends target/ to .gitignore
# → creates dev branch
# → writes CODEOWNERS
```

L5 always requires a manual browser step (Claude App install) followed by
a re-run of `conductr setup wizard` or `status`.

---

## Out of scope

- Language-agnostic checks (the CI check assumes Rust / `cargo test`)
- GitHub Enterprise variations
- Remote or non-git repositories
