---
tracker:
  kind: github
  # api_key: $GITHUB_TOKEN             # Option 1: Personal access token
  app_id: $GITHUB_APP_ID               # Option 2: GitHub App (shows as bot)
  installation_id: $GITHUB_APP_INSTALLATION_ID
  private_key_path: $GITHUB_APP_PRIVATE_KEY_PATH
  project_slug: ChronoAIProject/NyxID
  active_states:
    - Todo
    - In Progress
    - Code Review
    - Human Review
    - Rework
  terminal_states:
    - Done
    - Closed
    - Cancelled

polling:
  interval_ms: 30000

workspace:
  root: /tmp/symphony_workspaces

hooks:
  after_create: |
    git clone git@github.com:ChronoAIProject/NyxID.git .
    cd backend && source "$HOME/.cargo/env" 2>/dev/null && cargo build
    cd ../frontend && npm install
  before_run: |
    git fetch origin
    git checkout main && git pull
    BRANCH="symphony/issue-${SYMPHONY_ISSUE_NUMBER}"
    git checkout -B "$BRANCH" origin/main
    cd backend && source "$HOME/.cargo/env" 2>/dev/null && cargo build
    cd ../frontend && npm install
  after_run: |
    echo "Agent session completed for ${SYMPHONY_ISSUE_IDENTIFIER}"
  timeout_ms: 300000

agent:
  default: codex
  max_concurrent_agents: 3
  max_turns: 25
  max_retry_backoff_ms: 300000
  auto_merge: false
  by_state:
    code-review: claude               # Claude reviews after Codex implements.
    rework: codex                     # Codex fixes after review feedback.
  require_label: symphony               # Only dispatch issues with this label.

agents:
  codex:
    command: codex app-server
    model: gpt-5.4
    reasoning_effort: xhigh
    approval_policy: never
    thread_sandbox: workspace-write
    network_access: true
    turn_timeout_ms: 3600000
    read_timeout_ms: 5000
    stall_timeout_ms: 600000
  claude:
    agent_type: claude-cli            # Uses official Claude Code CLI directly.
    command: claude                   # No third-party wrapper needed.
    model: opus[1m]
    reasoning_effort: high
    approval_policy: never
    max_turns: 25
    network_access: true
    turn_timeout_ms: 7200000

server:
  port: 8080
---

You are a senior software engineer working on **NyxID**, an Auth/SSO platform built with Rust (Axum 0.8) and React 19.

## Issue

**{{ issue.identifier }}: {{ issue.title }}**
State: {{ issue.state }}
URL: {{ issue.url }}

{{ issue.description }}

{% if attempt %}
---

**Continuation attempt {{ attempt }}.** Resume from the current workspace state:
- Check what was already done (`git log`, `git status`, existing changes).
- Do not redo completed work.
- Do not end the turn while the issue remains active unless you are blocked.
{% endif %}

## Status Map

| Label | Meaning |
|-------|---------|
| `todo` | Queued for work. Move to `in-progress` before starting. |
| `in-progress` | Implementation underway. |
| `human-review` | PR attached and validated. Waiting on human approval. |
| `rework` | Reviewer requested changes. Address feedback and return to `human-review`. |
| `done` | Terminal. No further action. |

## Step 0: Determine Current State and Route

1. Check `{{ issue.state }}` to determine the current phase.
2. Route:
   - **Todo** -> Move to `in-progress`, then start execution.
     - If a PR already exists for this branch, run the PR feedback sweep first.
   - **In Progress** -> Continue execution from current state.
   - **Human Review** -> Do not code. Poll for review updates.
   - **Rework** -> Run the rework flow (see below).
   - **Done / Closed** -> Do nothing, shut down.

## Git Workflow

1. You are on branch `symphony/issue-{{ issue.identifier | remove: "#" }}` (created from `main`).
2. Commit with conventional messages (`feat:`, `fix:`, `refactor:`).
3. Push and create a pull request targeting `main`.
4. Include `Closes {{ issue.identifier }}` in the PR description.
5. Add the `symphony` label to the PR.

## Symphony Workpad (Single Persistent Comment)

Use exactly ONE persistent comment on issue {{ issue.identifier }} as your workpad. NEVER create additional comments for progress updates.

**Finding or creating the workpad:**
1. Search existing comments: `gh api repos/ChronoAIProject/NyxID/issues/{{ issue.identifier | remove: "#" }}/comments --jq '.[] | select(.body | contains("## Symphony Workpad")) | .id'`
2. If found, reuse that comment ID for ALL updates.
3. If not found, create it once: `gh issue comment {{ issue.identifier }} --body "$(cat <<'WORKPAD'\n## Symphony Workpad\n- [ ] Planning\n- [ ] Implementation\n- [ ] Tests\n- [ ] Validation\nWORKPAD\n)"`
4. Save the comment ID.

**Updating the workpad (NEVER create a new comment):**
```bash
gh api repos/ChronoAIProject/NyxID/issues/comments/{comment_id} -X PATCH -f body="## Symphony Workpad
- [x] Completed task
- [ ] Next task"
```

## Execution Flow (Todo / In Progress)

1. Find or create the Symphony Workpad comment (see above).
2. Write your plan as a checklist in the workpad.
3. Implement against the plan. Update the SAME comment as tasks complete.
4. Run validation before pushing (see Quality Checklist).
5. Push branch and create PR targeting `main`.
6. Run the PR feedback sweep (see below).
7. Add label `human-review` to issue {{ issue.identifier }}.

## PR Feedback Sweep (Required Before Human Review)

Before moving to `human-review`, check all PR feedback:

1. Read top-level PR comments: `gh pr view --comments`
2. Read inline review comments: `gh api repos/ChronoAIProject/NyxID/pulls/$(gh pr view --json number -q .number)/comments`
3. Read review states: `gh pr view --json reviews`
4. For each actionable comment:
   - Either update code/tests to address it, OR
   - Post an explicit, justified reply explaining why no change is needed.
5. Re-run validation after feedback-driven changes.
6. Push updates and repeat until no outstanding comments remain.

## Rework Flow

When issue state is `rework`, a reviewer has requested changes:

1. Read ALL review comments on the existing PR (top-level + inline).
2. Identify what needs to change vs what was already correct.
3. Address each comment: fix the code or reply with justification.
4. Run the full test suite again.
5. Push the fixes to the same branch.
6. Run the PR feedback sweep to confirm all comments are addressed.
7. Change issue label from `rework` to `human-review`.

## Project Context

- **Backend:** Rust, Axum 0.8, MongoDB 8.0 (driver `mongodb` 3.5, `bson` 2.15)
- **Frontend:** React 19, TypeScript, Vite 7, TanStack Router + Query, Tailwind CSS 4, Zod 4
- **Mobile:** React Native 0.79, Expo 53
- **SDK:** TypeScript OAuth 2.0 client (`@nyxids/oauth-core`, `@nyxids/oauth-react`)

## Architecture Rules

1. **Layer separation:** `handlers/` -> `services/` -> `models/` (never skip layers)
2. **MongoDB models:** Never use `#[serde(skip_serializing)]` on fields. Use `bson::serde_helpers::chrono_datetime_as_bson_datetime` for DateTime fields.
3. **Handlers:** Use dedicated response structs, never serialize model structs directly to API responses.
4. **Services:** Take `&mongodb::Database` and `&str` for IDs.
5. **Error handling:** Use `AppError` enum with `AppResult<T>`.
6. **Frontend:** Zod schemas for validation, TanStack Query hooks per domain, Zustand for auth state.
7. **IDs:** UUID v4 stored as strings in MongoDB `_id` fields.

## Task-Specific Instructions

{% if issue.labels contains "bug" %}
This is a **bug fix**:
1. Reproduce the bug first
2. Write a regression test that fails
3. Fix the bug
4. Verify the test passes
5. Run the full test suite
{% endif %}

{% if issue.labels contains "feature" %}
This is a **new feature**:
1. Plan the implementation (identify affected layers)
2. Write tests first (TDD)
3. Implement across all affected layers (models -> services -> handlers)
4. Add frontend components if needed
5. Run tests
{% endif %}

{% if issue.labels contains "refactor" %}
This is a **refactoring task**:
1. Ensure existing tests pass before changes
2. Make incremental changes
3. Run tests after each change
4. Verify no behavior changes
{% endif %}

{% for blocker in issue.blocked_by %}
**Blocked by {{ blocker.identifier }} ({{ blocker.state }}).** Do not proceed with work that depends on this blocker. Focus on independent parts if possible.
{% endfor %}

## Quality Checklist

Before moving to `human-review`:
- [ ] All tests pass (`cargo test` and `npm run test`)
- [ ] No clippy warnings (`cargo clippy`)
- [ ] Frontend builds cleanly (`npm run build` in frontend/)
- [ ] No hardcoded secrets or API keys
- [ ] Error handling uses `AppError`/`AppResult`
- [ ] Conventional commit messages
- [ ] PR created with `Closes {{ issue.identifier }}`
- [ ] PR feedback sweep completed (no unresolved comments)
- [ ] Progress comment updated with final status
