# NYX skill template

Status: Part 1 of #2331. Off by default behind the per-person feature flag
`assistant:skill-runs`.

The NYX skill template gives every skill a short, plain-language outline that
NyxBot can show to a person before doing the work, and later track while the
work runs. The outline is written by the skill's author and lives in the skill's
own `SKILL.md`, so no Ornn format change is needed.

## Format

After the YAML frontmatter, `SKILL.md` has these level-2 sections, in this order:

| Section | Required | Content |
| --- | --- | --- |
| `## Overview` | Yes | 2–3 short plain paragraphs: what the person ends up with, who it suits and who it doesn't, what it needs and what it changes. |
| `## Approach` | Yes | Numbered steps, one per line: `N. **Title.** One or two plain sentences.` |
| `## Needs and produces` | No | One `Needs:` line and one `Produces:` line. |
| `## Details` | No | Everything an agent needs to do the work. Any later sections are details too. |

A step may continue on indented lines. An indented line made only of tags is a
tag line, with tags separated by `·`:

- `Uses: <dependency>`: this step follows the named dependency's guidance.
- `Asks you`: this step needs the person's input.

Tags are display hints only. They never change approvals: action cards and the
access mode still decide what needs confirmation.

```markdown
## Overview
Give me a topic and a repository. I research the topic on the web, summarise
what I find with sources, and open one GitHub issue with that summary.

It suits turning a question into a tracked task. It is not for long reports
or for editing existing issues.

You need Firecrawl and GitHub connected. The only change is one new issue.

## Approach
1. **Agree the question.** I restate the topic and the repository so we aim at the same thing.
   Asks you
2. **Research it.** I search the web and read the most relevant pages.
   Uses: firecrawl-via-nyxid
3. **Draft the issue.** I write a title and a short body with sources, and show it to you.
   Asks you
4. **Open the issue.** I create it in the repository and send you the link.
   Uses: github-via-nyxid

## Needs and produces
Needs: Firecrawl, GitHub (write to issues).
Produces: one GitHub issue link.
```

A starter file is in [`skills/TEMPLATE.md`](../skills/TEMPLATE.md), and
[`skills/research-to-github-issue`](../skills/research-to-github-issue/SKILL.md)
is a complete combined example.

## Limits

Recommended (NyxBot's drafts get warnings, never refusals, when they miss these):

- 3–7 steps.
- Overview and Approach together under 2,000 characters.
- No tables or code in the Overview; put them under Details.
- Include Needs and produces.

Hard limits (enforced when reading). They count Unicode characters, not bytes:

| Field | Limit |
| --- | --- |
| Overview | 1,200 |
| Steps | 1–10 |
| Step title | 80 |
| Step text | 300 |
| `Needs` / `Produces` | 200 each |

The reader returns **no outline**, never a cut-down one, when a required section
is missing; a section appears twice or out of order; a heading is malformed; the
Approach contains anything other than numbered steps, continuation lines indented
one to six columns, and tag lines (a code fence at any indentation, or a line
deep enough to be an indented code block, refuses the outline); a line uses tag
syntax but is not a
valid tag line (for example two `Uses` tags, or a tag mixed with prose); a hard
limit is exceeded; or a `Uses` tag does not resolve (below). Headings inside
code fences don't count: a fence closes only with the same character, a run at
least as long as the opening one and nothing else on the line, and an
unterminated fence runs to the end. A skill without an outline keeps working as
an ordinary skill.

## Combining skills

1. Every skill, large or small, has its own Overview and Approach, so it makes
   sense on its own.
2. `Uses` must identify exactly one declared dependency, by name or GUID. Ornn
   declares dependencies in frontmatter `metadata.depends-on` as a YAML list
   of `<name-or-GUID>@<major.minor>` or `<name>@<dist-tag>` strings. NyxID
   parses the frontmatter as YAML, so only the `depends-on` key of the
   top-level `metadata` mapping counts; the same text inside another key, a
   quoted or block scalar or a nested mapping never does. Frontmatter that
   doesn't parse strictly (a repeated key; an alias, anchor or merge key; a
   `depends-on` that isn't a list of strings) declares nothing, so every
   `Uses` tag fails closed.
3. NyxID resolves each declared dependency to an exact GUID, version and
   SHA-256 when the skill is previewed and pinned (at most 16 resolved
   dependencies per skill; Ornn itself allows 50 declared). A `Uses` tag matches
   a pin in that verified closure; it is never re-resolved by name at run time,
   so a dist-tag that moves later changes nothing until an explicit re-pin. If a
   `Uses` tag matches no pin, more than one pin, or a pin the frontmatter does
   not declare, the skill has no outline.
4. Combining means following the smaller skill's guidance inside the same run,
   with the same agent, authority and approvals. It never hands work to another
   agent or adds grants.
5. The parent step says in plain words what it passes to the smaller skill and
   what it expects back, matching the smaller skill's Needs and Produces. The
   parent step is done when that work is done; if it fails or is skipped, the
   parent step fails or is skipped too.
6. Display is one level deep: the parent's steps are shown, and a smaller
   skill's own steps are read when that step starts. Nested reads stay inside the
   set pinned with the original skill.

## How NyxID uses it

Everything below requires `assistant:skill-runs` for the acting person and
applies to owner turns only; guests never see outlines. With the flag off,
NyxBot's instructions, tool list and tool results are unchanged.

- **Reading.** `skill_read` accepts `section: "outline"` for an attached skill or
  one of its pinned dependencies. The result is a page of the rendered outline,
  marked `untrusted_guidance`, within the usual tool-response budget; long
  outlines continue with `next_offset`, echoing the page's
  `outline_fingerprint`. If a dependency was re-pinned between pages, the
  answer is `outline_changed` and the read restarts at `0`, never a page of a
  different outline. A skill without an outline returns
  `outline_available: false`.
- **Previewing.** `preview_agent_skill` adds an `outline` page for the verified
  root package, so NyxBot can explain a skill before it is attached. When the
  preview and the outline don't both fit one tool result, `outline` says so
  instead of carrying content; when even that notice can't fit, the result is
  the plain preview, exactly as with the flag off. Calling again with
  `outline_offset` returns an outline-only page (start at `0`). Pages after the
  first must echo the `outline_fingerprint` of the page they continue, and a
  deferred first page should echo the preview's fingerprint: if a dependency
  tag moved between calls, the answer is `outline_changed` instead of an
  outline for different pins. Offset `0` without a fingerprint is an
  intentional fresh read; refresh the full preview to see its resolved pins.
- **Showing it in the app.** The app's catalog preview
  (`GET /assistant/nyxagent/skills/catalog?skill=&version=`) adds two fields:
  `outline`, the parsed outline as structured steps (`null` when the skill has
  none), and `author`, who published it in Ornn. A system skill or a skill
  published by a NyxID service account belongs to the platform; otherwise the
  publishing account is shown as an organization or a person, and an account
  NyxID doesn't know gets no author. The Find skills version view shows them as
  a skill card. Flag off, the route returns exactly today's preview.
- **Explaining.** NyxBot's instructions ask it to read the outline before
  multi-step or changing work a skill covers, explain it in plain words, and ask
  whether to go ahead. Agreeing is not an approval.
- **Drafting.** When `assistant:agent-learning` is also on, NyxBot drafts follow
  the template, and `draft_agent_skill` results may include `outline_warnings`
  with fixed codes (`missing_overview`, `missing_approach`, `step_count`,
  `outline_length`, `overview_table_or_code`, `missing_needs_produces`,
  `uses_unresolved`, ...). Warnings are reported after the draft is created and
  never refuse it; existing validation keeps its refusals. NyxBot is told not to
  draft or publish again because of them. Drafts can't declare dependencies yet, so combined skills
  are written and published by maintainers through the existing Ornn flow.

Outline text is untrusted reference data, like every other skill read: a
matching hash proves which package it came from, not that its text is safe. It
never grants permissions, approves actions or changes the access mode. Nothing
is stored on the agent; the outline is read from the hash-verified package each
time, and no audit or log entry contains outline text.

## Capability skills and skillsets

Write reusable skills at three levels, none of them tied to a business:

- **Capability skills** do one thing with one provider through NyxID, such as
  `sms-send-twilio` or `records-notion`. Each states its success signal.
- **Behaviour and workflow skills** describe how to act or a procedure, such as
  `caller-details-readback` or `booking-with-availability`, without naming a
  provider.
- **Skillsets** (Ornn) bundle the members for one use case. Their master prompt
  follows this template: the Overview says which use case the skillset is for,
  and each Approach step's `Uses` names a member.

Business facts live in a separate facts pack. NyxBot pins skills only today;
presenting a skillset as one skill (its master prompt as `SKILL.md`, its members
as the dependencies) is proposed for a later part of #2331. See
[the examples](skill-templates/README.md), which also explain how to check
templates and preview their cards locally.

## Rollout

`assistant:skill-runs` is off by default for everyone. Upgrade every backend
replica before enabling any override, then pilot with a personal override.
Turning the flag off restores the previous behavior on the next tool listing and
turn; it changes no stored data. Changing the flag changes NyxBot's stable
instructions, so an existing conversation's upstream context resets once.

Later parts of #2331 add skill cards in chat (suggest or start a skill) and a
live step checklist under the same flag; this part ships the card and the
checklist components and shows the card in the Find skills preview.
