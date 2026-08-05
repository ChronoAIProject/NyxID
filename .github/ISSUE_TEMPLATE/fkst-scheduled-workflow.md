---
name: FKST scheduled workflow
about: Run a workflow on a schedule — once, or on a cron cadence. No extra setup.
title: "[scheduled] "
labels: ["fkst-scheduled-workflow"]
---

<!--
A scheduled workflow runs a workflow definition from this repository on a
schedule. Everything it needs is on this issue — there is no Actions workflow, no
CLI, and nothing to install.

BEFORE YOU FILE:

1. Assign EXACTLY ONE person: the creator of the fkst session that will run it
   (the person who opened its `fkst-substrate-trigger` issue). That assignee is
   the routing key — zero or several assignees means no session to run it.
2. The workflow id below must resolve to `.fkst/workflows/<id>.toml` in this
   repository.

UNLIKE a session trigger issue, THIS BODY STAYS EDITABLE. Change the cadence or
the arguments whenever you like; the next reconcile picks the edit up. A cadence
nobody could change would not be a feature.

Two things to know:

- The minimum cadence is set by the deployment (15 minutes by default). Every
  firing creates a run issue and boots a session pod, so a tighter cadence is
  REJECTED with a message naming the limit rather than silently slowed.
- To pause without closing this issue, add the `fkst-cron-paused` label. Remove it
  to resume. Closing the issue stops the schedule for good.

NEVER put a secret, token, or credential in `### Arguments`. This body is visible
to everyone who can read the repository, and its arguments are copied into every
run issue. Secrets reach a step through a named environment profile, referenced
from the workflow definition by key name.
-->

### Workflow

<!-- The workflow id: `.fkst/workflows/<id>.toml` in this repository. -->

my-workflow

### Run Mode

<!--
Either:
  once                 run one time, as soon as this issue is observed
  cron: <expression>   a standard five-field UTC cron expression

Examples:
  cron: 0 3 * * *      daily at 03:00 UTC
  cron: 0 1 * * 1-5    weekdays at 01:00 UTC
  cron: */30 * * * *   every 30 minutes

Day-of-week is 0-6 with 0 = Sunday. `7` is rejected rather than treated as
Sunday, because it is far more often a typo in the month field next to it.
-->

cron: 0 3 * * *

### Arguments

<!--
Optional `key: value` lines passed to the workflow's steps as DATA — they are
escaped, never interpolated into a shell command. Delete this section if the
workflow takes none.

Keys start with a letter and contain letters, digits and underscores.
-->
