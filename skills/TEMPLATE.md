---
name: your-skill-name
description: One or two sentences on what the skill does and when an agent should use it.
version: "1.0"
license: MIT
metadata:
  category: plain
  tag:
    - nyxid
  # Only for combined skills: every `Uses:` tag below must name one of these.
  # depends-on:
  #   - smaller-skill@1.0
---

# Your skill title

<!--
NYX skill template (docs/SKILL_TEMPLATES.md). Keep Overview and Approach plain
and short: under 2,000 characters together, 3-7 steps, no tables or code.
Hard limits: overview 1,200 characters, 10 steps, step title 80, step text 300.
Delete this comment.
-->

## Overview
What the person ends up with, in one or two sentences.

Who it suits, and who it doesn't.

What it needs (connections, access) and what it changes.

## Approach
1. **First step.** One or two plain sentences on what happens and why.
   Asks you
2. **Second step.** One or two plain sentences.
   Uses: smaller-skill
3. **Third step.** One or two plain sentences.

## Needs and produces
Needs: the connections or access the skill requires.
Produces: what the person gets at the end.

## Details
Everything an agent needs to do the work: commands, API calls, edge cases,
formats, failure handling. For a combined skill, say for each `Uses` step what
it passes to the smaller skill and what it expects back.
