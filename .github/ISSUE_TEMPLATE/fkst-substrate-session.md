---
name: fkst substrate session
about: Start an fkst agent session against this repository.
title: "[session] "
labels: [fkst-substrate-trigger]
---

<!--
Opening this issue starts an fkst session. fkst parses the sections below by
their EXACT `### ` headings. Do NOT rename the headings. Do NOT put secrets,
tokens, or credentials anywhere in this issue — they are supplied only through
your environment and are never read from issue text.

⚠️ ONE-SHOT CONFIG: the moment the session registers (the 🟢 comment appears),
EVERYTHING in this issue body is FROZEN. Edits after that are rejected with an
`fkst-config-rejected` label and change nothing. To change any setting, close
this issue and open a new trigger.

TWO KINDS OF SETTING. `### Session Configuration` and `### Package Configuration`
below are DIVIDERS: they group what follows and are otherwise ignored, so
deleting one changes nothing. Session settings describe THIS session — who drives
it, which branches it moves, what it emits — and mean the same thing whichever
packages run. Package settings belong to a package and are meaningless without it.

BRANCHES: To override branch topology, add an optional `### Source Branch` section
with exactly one existing upstream branch (default: the repository default), and/or
an optional `### Target Branch` section with exactly one integration branch
(default: `fkst-hosted-default`). Add the heading only WITH a value — an empty
branch section is rejected. fkst creates a missing target at the source head before
the session starts and never resets an existing target. Work branches start from the target branch
and pull requests merge into it; completed target work rolls up into the
source. Source and target may be the same branch, which disables the rollup step.

CREATOR AND AUTHORITY: For a human-authored trigger, the session creator is the
issue author. For an App-authored trigger, the trigger's sole assignee is the creator.
The creator must be a deployment global admin or have admin or maintain permission
on this repository; the deployment access allowlist also applies.

LABEL ISOLATION AND ROUTING: One creator's open triggers must have pairwise-disjoint
effective work-label sets; different creators may reuse the same labels. Every work
issue needs exactly one assignee: this session's creator. Its author must be the
creator, a login under `### Session Collaborators`, or a deployment global admin.
-->

### Session Configuration

### Session Name

<!-- One line. Lower-case DNS-label: ^[a-z0-9]([a-z0-9-]*[a-z0-9])?$, 1-40 chars. -->
my-first-session

### Environment

<!--
Optional. One line naming an environment you created via
`PUT /api/v1/users/me/environments/<name>`. Delete this whole section to run with
no environment.
-->

### Auto-merge

<!--
Optional. Set to `true` to have fkst automatically merge THIS session's pull
requests into the target branch as soon as GitHub reports them mergeable. This
BYPASSES review and status checks — use it only on trusted repos. Accepted truthy
values: true / yes / on / enabled / 1 (case-insensitive). Anything else, or
deleting this section, leaves auto-merge OFF (the default).
-->
false

### FKST Contributors

<!--
Optional LOG-DOWNLOAD list. Add GitHub logins or numeric user ids — one per line,
or comma/space separated; a leading `@` is fine — that may download this session's
redacted logs. A human-authored trigger's creator is its author and already has log
access; an App-seeded trigger lists its assigned creator here. Deployment global
admins can always download. These entries do NOT grant work-item or package-content
authority; use `### Session Collaborators` for that. Delete this whole section when
no additional log viewers are needed. (The old heading `### Log Access Allowlist`
is still accepted as an alias.)
-->

### Session Collaborators

<!--
Optional WORK-ITEM AUTHORITY list. List extra GitHub logins — one per line, or
comma/space separated; a leading `@` is fine — granted authority over THIS
session's work issues in addition to its creator and deployment global admins.
These users may raise, label, and comment on the session's work issues and their
logins are admitted by the package-side author policy. Repository admins do not gain
this authority unless they are also the creator, listed here, or a deployment global
admin. Delete this whole section to grant no additional authority. (Frozen at
registration like every other setting in this issue.)
-->

### Output Language

<!--
Optional. ONE locale tag (e.g. `en`, `zh`, `zh-CN`) — the language the session's
packages write user-visible comments in. The value must EXACTLY match a
`locales/<value>.lua` file shipped by the session's package (case- and
separator-sensitive); a mismatch silently falls back to English. Delete this
whole section for English (the default).
-->

### Work Label

<!--
Optional. Delete this whole section to auto-detect the work label(s) from your
packages' `[github].work_labels` — a session with no explicit label is driven by
every label its packages declare. Otherwise name ONE GitHub label whose OPEN issues
are this session's work queue: max 50 characters, no commas. Create work items as
separate issues carrying this label (see the "fkst work item" template). Your open
sessions' effective label sets must not overlap each other, but a different creator's
sessions may use the same labels. Give every work issue exactly one assignee equal to
this session's creator; zero, multiple, or another assignee is `fkst-unrouted`.
-->

### Engine Config

<!--
Optional. Advanced ENGINE-wide tunables, ONE `KEY=value` per line, drawn ONLY from
this allowlist (anything else is rejected with an explanatory comment). Settings
that belong to a PACKAGE go in `### Package Env` below, not here:
  FKST_LLM_MODEL=<model id>                run THIS session on a different model
    served by the deployment's LLM endpoint (letters, digits, . _ / : - only)
  FKST_LLM_REASONING_EFFORT=<tier>         minimal | low | medium | high | max
    (deployment default: max)
  FKST_CODEX_PERMIT_SLOTS=<1..32>          concurrent codex subprocesses
  FKST_QUEUE_CAPACITY=<1..1024>            per-queue capacity
  FKST_MAX_IN_FLIGHT_PER_DEPT=<1..1024>    per-department concurrency
  FKST_DURABLE_ADMISSION_BURST_PER_DEPT=<1..1024>
  FKST_RETRY_DEFAULT_MAX_ATTEMPTS=<1..100>
  FKST_RETRY_DEFAULT_BASE / FKST_RETRY_DEFAULT_CAP /
  FKST_DEPARTMENT_DEFAULT_STALL_WINDOW /
  FKST_SUBSCRIBER_ABSENT_DELIVERY_BUDGET=<n>s|m|h   (1 second ..= 7 days; the
    effective retry CAP must stay >= the effective BASE — defaults 60s / 30m)
  FKST_RATE_POOL_<NAME>=<burst>,<refill_per_minute> throttle a program by
    basename (e.g. FKST_RATE_POOL_GH=10,10); platform defaults can only be
    TIGHTENED, never widened.
Delete this whole section for the engine defaults.
-->

### Package Configuration

### Packages

<!--
One package reference per line, each as `owner/repo@ref:path/to/package`, pointing
at a PUBLIC repo that has an `fkst.toml` at that path. At least one package source is
required across `### Packages` and `### Manifest`; this section may be empty or deleted
when a manifest supplies the packages.
-->
ChronoAIProject/fkst-hosted@packages:packages/workflow-dev

### Manifest

<!--
Optional. One `owner/repo@ref:path` per line pointing at a fkst-manifest JSON (a
reusable package bundle) — its packages are added to any you list in `### Packages`.
A manifest may also supply package settings; anything you set below overrides it.
Delete this section if unused.
-->

### Package Env

<!--
Optional. Settings for a specific PACKAGE, grouped under `#### <package>` lines,
then one `FKST_KEY=value` per line. `<package>` is the last path segment of a
package reference — `github-devloop` for `…@packages:packages/github-devloop`.

Precedence: the package's own default, then a manifest's `packageEnv`, then what
you write here (which wins, per key).

A key may be configured by only one package, and platform-owned names
(FKST_SESSION_*, FKST_GITHUB_*, FKST_TRIGGER_ISSUE, …) are rejected.

Example — let the devloop refine a blocked specification itself instead of
stopping for a human, allowing two attempts (0, the default, keeps it stopping):

#### github-devloop
FKST_DEVLOOP_AUTO_REFINE_MAX=2
-->
