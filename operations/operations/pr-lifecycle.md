# PR lifecycle

A PR enters the world when an issue is triggered and ends when it merges into
`develop` (or is abandoned). This is the canonical path; deviations are
documented in [`safety.md`](./safety.md) under "Failure modes".

## Stages

### 1. Trigger

The orchestrator comments on each unblocked, non-`human` issue:

```
gh issue comment <N> --body "@claude please implement"
```

All issues in a batch are triggered in parallel. If a batch has more than 3
unblocked issues and no ARNs exist yet, the architect agent runs first — see
[`architecture-reference-notes.md`](./architecture-reference-notes.md).

### 2. Wait for the PR

The orchestrator polls every 60 s, up to 30 min per issue. PRs are matched to
issues by branch name (`claude/issue-<N>-*`) or by issue mention in the PR
body. Cadence details: [`../cadence/orchestration-cycle.md`](../cadence/orchestration-cycle.md).

### 3. Wait for CI

```
gh pr checks <number> --watch
```

A failing check ends this branch of the lifecycle and routes to the failure
playbook in [`safety.md`](./safety.md).

### 4. Review

**Check for an existing review first.** The `claude-code-review.yml` workflow
auto-reviews new PRs. Before doing your own:

```
gh pr reviews <number> --json author,body,state
```

If an automated review is present and looks good, summarise it for the user
and proceed to merge. Run your own review only when:

- no automated review exists yet, or
- ARN compliance specifically needs checking
  ([`architecture-reference-notes.md`](./architecture-reference-notes.md)).

A manual review reads the diff (`gh pr diff <number>`) and checks it against
the issue's acceptance criteria and ARN.

### 5. Merge

When CI passes and review is approving:

```
gh pr merge <number> --squash --delete-branch
```

Always squash. Always delete the branch.

### 6. Branch hygiene

After every merge, sync the local development branch:

```
git checkout develop && git pull origin develop
```

(The original poorchestrator skill says `dev`; this repo uses `develop`. See
the targeting rule in [`safety.md`](./safety.md).)

### 7. Re-classify

A merge often unblocks downstream issues. The orchestrator re-runs
classification before the next pass — see
[`dependency-resolution.md`](./dependency-resolution.md).

## What does NOT happen here

- Force-pushing — never.
- Force-merging through failing CI or merge conflicts — never; the user is
  asked. See [`safety.md`](./safety.md).
- Running deployment commands — orchestration covers implementation only.
  Deployment is a human responsibility.

## Skills

- [`crates/conductr-orchestrate`](../../crates/conductr-orchestrate) — Rust
  implementation of stages 1–7.
- [`vendor/poorchestrator/SKILL.md`](../../vendor/poorchestrator/SKILL.md) §§
  "Workflow / A3. Execute the cycle" and "B4. Execute in batches".
