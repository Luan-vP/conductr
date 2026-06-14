---
name: orchestrate
description: Manage GitHub issue implementation by delegating to the Claude GitHub Action bot. When called with no arguments, automatically finds unblocked issues, triggers them in parallel, reviews and merges blocking PRs, and repeats until all issues are done. When called with issue numbers or a label, processes that specific batch. Respects dependency ordering throughout.
cli: conductr orchestrate --repo <owner/repo> [--dry-run] [--once] [--poll-secs <n>] [--human-assignee <user>] [--ci <mode>] [--repo-root <path>]
---

# Orchestrate Issue Implementation

Manage GitHub issue implementation by delegating to the Claude GitHub Action bot, monitoring PRs, and merging in dependency order.

## Input

`$ARGUMENTS` — optional. Three modes:

- **No arguments** (`/orchestrate`) — auto-mode: scan all open issues, find what's unblocked, trigger it, review blocking PRs, repeat
- **Issue numbers** (`/orchestrate 20 21 22`) — process a specific batch in dependency order
- **Label** (`/orchestrate --label ready-to-implement`) — process all open issues with that label

Add `--no-architect` to skip the architecture pass.

## Determining the human assignee

When a `human`-labeled issue needs to be assigned, determine the most appropriate person using this priority order:

1. **CODEOWNERS file** — check `.github/CODEOWNERS`, `CODEOWNERS`, or `docs/CODEOWNERS` for path-based ownership rules. Match the files likely affected by the issue (infer from the issue title/body) to the most specific matching pattern. Use the first owner listed for that pattern.
2. **Repo owner** — if no CODEOWNERS file exists or no pattern matches, determine the repo owner:
   ```
   gh repo view --json owner --jq '.owner.login'
   ```
3. **Repo admins** — if the owner is an organization, find an admin collaborator:
   ```
   gh api repos/{owner}/{repo}/collaborators --jq '.[] | select(.permissions.admin==true) | .login' | head -1
   ```

Cache the resolved username for the duration of the orchestration session — don't re-resolve for every issue.

## Workflow

### Mode A: No arguments (auto-mode)

This is the continuous orchestration loop. It looks at the current state of all open issues and PRs, figures out what can move forward right now, and does it.

#### A1. Survey the landscape

Gather the full picture:

```
gh issue list --state open --json number,title,body,labels
gh pr list --state open --json number,title,body,headRefName,statusCheckRollup
```

For each open issue, read its body and determine:
- What it depends on (parse `depends on #N`, `blocked by #N`, `after #N`, `requires #N`)
- Whether it already has an open PR (check for PRs with branch pattern `claude/issue-<N>-*` or that mention the issue)
- Whether it's already been triggered (check issue comments for `@claude please implement`)

For each open PR, determine:
- Which issue it's for (from branch name or body)
- CI status (passing, failing, pending)

#### A2. Classify issues

Put each open issue into one of these buckets:

| Bucket | Condition | Action |
|--------|-----------|--------|
| **Ready** | All dependencies are closed/merged AND no open PR AND not yet triggered | Trigger implementation |
| **PR open** | Has an open PR | Check CI, review, merge if ready |
| **PR failing** | Has an open PR with failing CI | Report failure, offer to request a fix |
| **Blocked** | Has open dependencies (issues not yet closed) | Skip for now |
| **Triggered, waiting** | Has been triggered but no PR yet | Poll, wait |
| **Human** | Has the `human` label | Do not trigger — requires human action. Assign to the resolved human assignee (see above). Blocks downstream issues until closed. |

Present the landscape to the user:

```
Issue Landscape
===============
Ready to start:
  #A — <title>
  #B — <title>

PRs ready to review:
  #C → PR #P1 (CI passing)

Blocked:
  #D — Depends on #A, #C

Waiting for bot:
  (none)

Needs human action:
  #E — <title> [human]

Plan: Review and merge PR #P1, then trigger #A and #B in parallel.
```

#### A3. Execute the cycle

**First, clear the path** — if any PRs are blocking downstream issues:

1. Check for an existing CI-triggered review first. The `claude-code-review.yml` workflow automatically reviews new PRs. Before doing your own review:
   ```
   gh pr reviews <number> --json author,body,state
   ```
   If a review already exists, read it and summarize the findings to the user. Only do your own review if no automated review is present yet or if you need to check ARN compliance specifically.

2. Review each blocking PR (if needed):
   - Read the diff with `gh pr diff <number>`
   - Check it against the issue's acceptance criteria
   - If the issue has an ARN, check architectural compliance
3. If CI passes and the review looks good (either from the automated review or your own), merge:
   ```
   gh pr merge <number> --squash --delete-branch
   git checkout develop && git pull origin develop
   ```
4. This may unblock new issues — re-classify after each merge.

**Then, trigger unblocked issues** — dispatch all ready issues in parallel using `runner_for` (see B4 for the full helper definition).

**Skip `human`-labeled issues** — issues with the `human` label require human intervention and must never be triggered. Assign them to the resolved human assignee:
```
gh issue edit <N> --add-assignee <resolved-assignee>
```
They remain in the dependency graph: downstream issues stay blocked until the `human` issue is closed manually. Report these to the user so they know what manual work is needed to unblock progress.

For each ready non-human issue, apply `runner_for(issue)`:
- `runner = web` → `gh issue comment <N> --body "@claude please implement"`
- `runner = tmux` → `conductr run-task --issue <N>`

If there are more than 3 unblocked issues and no ARNs exist yet, run the architect agent first (see Architecture Pass below).

**Then, wait and poll** — check every 60 seconds for new PRs from triggered issues. As PRs appear:
- Wait for CI
- Check for existing CI-triggered review before reviewing yourself
- Merge if CI passes and review looks good
- Re-classify to find newly unblocked issues

**Repeat** until all issues are closed or only blocked issues remain.

#### A4. Report

After each cycle, print what happened:

```
Cycle complete
==============
Merged: PR #P1 (for #C)
Triggered: #A, #B
Waiting: #A (PR pending), #B (PR pending)
Blocked: #D (waiting on #A, #B)
Human action needed: #E — <title> [human] (assigned to @<resolved-assignee>)

Next cycle will start when PRs arrive.
```

After reporting each cycle, run the calendar sync (see § Calendar Sync below).

### Mode B: Explicit issue list

When called with issue numbers or a label.

#### B1. Resolve issues

If arguments are issue numbers, use them directly. If `--label` is provided:
```
gh issue list --label "<label>" --state open --json number,title
```

List the issues and proceed.

#### B2. Architecture pass (recommended)

Before triggering any implementation, invoke the **architect** agent to generate Architecture Reference Notes (ARNs) for all issues in the batch:

```
Use the architect agent to analyze the batch of issues and generate ARNs
```

The architect will read all issue bodies, explore the codebase, build the dependency graph, and comment an ARN on each issue. Skip with `--no-architect`.

#### B3. Build dependency graph

Read each issue body with `gh issue view <N> --json body,title`.

Scan for dependency declarations (case-insensitive):
- `depends on #N` / `depends on #N, #M`
- `blocked by #N`
- `after #N`
- `requires #N`
- Checklist items like `- [ ] #N must be done first`

Also parse the Local Map from any existing ARN comment — it may encode dependencies visually.

Build a DAG. Validate:
- **No cycles** — report and ask the user to clarify.
- **Dependencies outside batch** — check if already closed. If open and not in batch, warn and ask.

Present the execution plan:

```
Execution plan:
  Batch 1 (parallel): #A — <title>, #B — <title>
  Batch 2 (after batch 1): #C — <title> (depends on #A)
  Batch 3 (after batch 2): #D — <title> (depends on #B, #C)

```

#### B4. Execute in batches

For each batch in the topological order:

Before triggering, filter out issues with the `human` label. Assign them to the resolved human assignee and list them separately:

```
gh issue edit <N> --add-assignee <resolved-assignee>
```

```
Needs human action (not triggered, assigned to @<resolved-assignee>):
  #X — <title> [human]

These issues block downstream batches until closed manually.
```

**Determine the runner for each issue** using `runner_for(issue)` before triggering:

```
runner_for(issue):
  labels ← gh issue view <N> --json labels --jq '[.labels[].name]'
  if "runner/tmux" in labels  → runner = tmux
  if "runner/web"  in labels  → runner = web
  arn ← most recent ARN comment on the issue (look for "## Architecture Reference Note")
  if arn contains "**Runner:** tmux" → runner = tmux
  if arn contains "**Runner:** web"  → runner = web
  runner = web   # default
```

**Trigger all non-human issues in the batch** simultaneously. Use the `runner` value to select the dispatch path:
- `runner = web` → trigger via GitHub issue comment (standard bot dispatch):
  ```
  gh issue comment <N> --body "@claude please implement"
  ```
- `runner = tmux` → open (or reuse) a local tmux session and run Claude there:
  ```
  conductr run-task --issue <N>
  ```

**Poll for PRs** — check every 60 seconds, up to 30 minutes per issue.

Look for PRs referencing each issue:
```
gh pr list --state open --json number,title,body,headRefName
```
Match by branch name `claude/issue-<N>-*` or issue mention in PR body.

**As each PR arrives:**

1. Wait for CI:
   ```
   gh pr checks <number> --watch
   ```

2. Check for an existing CI-triggered review before reviewing yourself:
   ```
   gh pr reviews <number> --json author,body,state
   ```
   The `claude-code-review.yml` workflow automatically reviews new PRs. If a review already exists, summarize its findings to the user. Only do your own review if no automated review is present or if ARN compliance needs checking.

3. If no existing review, review the diff against acceptance criteria and ARN.

4. If CI passes and the review looks good, merge:
   ```
   gh pr merge <number> --squash --delete-branch
   git checkout develop && git pull origin develop
   ```

5. Mark the issue as completed in the tracker.

**After all PRs in the batch are merged**, proceed to the next batch. The next batch's issues now have their dependencies on `dev`.

#### B5. Report final status

```
Orchestration Complete
======================
| Issue | Title       | PR   | Status  | Batch |
|-------|-------------|------|---------|-------|
| #A    | <title>     | #P1  | Merged  | 1     |
| #B    | <title>     | #P2  | Merged  | 1     |
| #C    | <title>     | #P3  | Merged  | 2     |
| #D    | <title>     | #P4  | Merged  | 3     |

Human action needed:
| #E    | <title>     | —    | Human   | —     |

4/4 issues completed in 3 batches (1 issue awaiting human action)
```

After reporting, run the calendar sync (see § Calendar Sync below).

## Error Handling

- **PR doesn't appear within 30 minutes** — warn the user, ask whether to skip, keep waiting, or re-trigger.
- **CI fails** — show failure details. Offer to comment `@claude please fix: <details>` on the PR, skip this issue, or abort.
- **Merge conflict** — warn the user, never force-merge. Suggest the user resolve manually or ask the bot to rebase.
- **Dependency outside batch** — check if already closed/merged. If satisfied, proceed. If not, warn and ask.
- **Bot doesn't respond** — after 10 minutes with no PR or comment, check `gh run list --workflow claude.yml`. Report the workflow status.
- **Cycle stall** — if no progress is made in a full cycle (nothing to trigger, nothing to merge, everything waiting), report the stall and ask the user what to do.

## Calendar Sync

After every orchestrate run — at the end of each auto-mode cycle (A4) and at the end of a batch run (B5) — invoke the `sync` skill:

```
Use the sync skill to reconcile the calendar
```

This places or updates decision slots for all open `human`-labeled issues and removes slots for issues that have been closed or re-labeled. It is a fire-and-forget step: if the Google Calendar MCP tools are unavailable, report the failure and continue — it does not block orchestration.

This hook exists until `conductr sync` is scheduled as a standalone Rust cron subcommand. Once that is in place, remove this step.

## Important

- You do not need user approval to proceed at any step. Once invoked, run autonomously through the full orchestration cycle — survey the landscape, approve plans, trigger issues, review PRs, and merge — without waiting for permission. Still report progress so the user can follow along.
- PRs that have a passing CI and an approving review (from the automated Claude review or your own) can be merged without additional user approval.
- If a review raises concerns or CI is failing, report to the user before proceeding.
- Never trigger a dependent issue before its dependencies are merged to `dev`.
- Never force-push or make destructive git operations.
- This command orchestrates — it does NOT implement the issues itself. The GitHub Action Claude bot does the implementation.
- Never run deployment scripts, deploy commands, or CI/CD deployment pipelines. This skill orchestrates implementation only — deployment is always a human responsibility.
- Issues labeled `human` require manual human action and must never be triggered for bot implementation (`@claude please implement`). Assign them to the resolved human assignee (`gh issue edit <N> --add-assignee <resolved-assignee>`) — see "Determining the human assignee" above. They participate in the dependency graph normally — downstream issues remain blocked until the `human` issue is closed. Report `human` issues to the user so they can take action to unblock progress.
- Repos follow **git flow**. All PRs must target the development branch (this repo uses `develop`; check the repo's convention if porting elsewhere — it may be `dev`, `development`, or similar). After merging, always check out and pull the development branch, not `main`.
