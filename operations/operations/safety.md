# Safety

The invariants that constrain everything in [`pr-lifecycle.md`](./pr-lifecycle.md)
and [`dependency-resolution.md`](./dependency-resolution.md), the policy for
issues that need a human, and the failure-mode playbooks.

## Invariants (never violated)

| Rule                                                              | Why                                                       |
| ----------------------------------------------------------------- | --------------------------------------------------------- |
| **Never force-push.**                                             | Destroys history other people may have based work on.     |
| **Never force-merge** (no overrides on failing CI or unresolved review). | The gates exist to keep `develop` healthy.            |
| **Never run deployment commands or pipelines.**                   | Orchestration covers implementation only; deploy is human.|
| **PRs target the development branch** (`develop` here, `dev` upstream). | Git flow — `main` tracks released code.            |
| **After merging, sync the development branch**, not `main`.       | Otherwise downstream issues build on stale state.         |
| **Never trigger an issue before its dependencies are merged**.    | Implementing agents need their inputs to actually exist.  |
| **Never trigger a `human`-labeled issue.**                        | See "Human issues" below.                                 |

These rules apply to humans and agents alike. CI and branch protection
enforce some of them; the rest are honour-system inside the orchestrator.

## Human issues

An issue with the `human` label cannot be implemented by the bot. Common
causes: secrets rotation, contract changes, vendor approvals, anything
needing a real signature.

The orchestrator must:

1. **Never** comment `@claude please implement` on a `human`-labeled issue.
2. **Assign** the issue to a resolved human (priority order):
   1. Match the most-likely-affected files against `.github/CODEOWNERS`,
      `CODEOWNERS`, or `docs/CODEOWNERS`. Use the first owner of the most
      specific matching pattern.
   2. Fall back to the repo owner: `gh repo view --json owner --jq '.owner.login'`.
   3. If the owner is an org, fall back to a repo admin:
      `gh api repos/{owner}/{repo}/collaborators --jq '.[] | select(.permissions.admin==true) | .login' | head -1`.
3. **Cache** the resolved username for the rest of the orchestration session
   — don't re-resolve per issue.
4. **Keep** the issue in the dependency graph: downstream issues stay
   `Blocked` until the human issue is closed manually.
5. **Report** human issues prominently in the cycle status block so the user
   sees what manual work is needed to unblock progress.

```
gh issue edit <N> --add-assignee <resolved-assignee>
```

## Failure modes

The standard situations and what the orchestrator does about each. None of
these are "stop the world" — most are "ask the user".

| Situation                               | Response                                                                                            |
| --------------------------------------- | --------------------------------------------------------------------------------------------------- |
| **PR doesn't appear within 30 min**     | Warn the user. Offer: skip, keep waiting, or re-trigger.                                            |
| **CI fails on an open PR**              | Show failure details. Offer: comment `@claude please fix: <details>`, skip this issue, or abort.    |
| **Merge conflict**                      | Warn the user. Never force-merge. Ask the user to resolve manually or ask the bot to rebase.        |
| **Bot doesn't respond after 10 min**    | Inspect `gh run list --workflow claude.yml` and report the workflow run status to the user.        |
| **Cycle stall** (a full pass made no progress) | Report and stop spinning. Ask the user how to proceed.                                       |
| **Out-of-batch open dependency**        | Check whether it's actually closed/merged. If not, warn and ask before continuing.                  |
| **Cycle in the dependency graph**       | Report the cycle and ask the user to clarify by editing issue bodies.                               |

## Skills

- [`crates/conductr-orchestrate`](../../crates/conductr-orchestrate) — runtime
  enforcement of the invariants and failure-mode playbooks.
- [`vendor/poorchestrator/SKILL.md`](../../vendor/poorchestrator/SKILL.md)
  §§ "Determining the human assignee", "Error Handling", and "Important".
