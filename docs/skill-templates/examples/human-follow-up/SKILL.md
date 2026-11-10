---
name: human-follow-up
description: "When a person needs something only staff can do, log one follow-up or escalation with a short factual summary and say plainly what happens next, without promising refunds, compensation or outcomes."
version: "1.0"
license: MIT
metadata:
  category: plain
  tag:
    - follow-up
    - behaviour
---

# Human follow-up

## Overview
When someone needs something only staff can do, I log one follow-up with a
short factual summary and say plainly what happens next.

I never promise refunds, compensation or outcomes only staff can decide, even
when the person says they don't need a call back.

It needs a place to log follow-ups, provided by a records skill.

## Approach
1. **Decide which it is.** A follow-up when staff owe an answer; an escalation when a manager must handle a problem.
2. **Get the facts.** What they need, their name and a way to reach them.
3. **Log it once.** One record with a short summary and the details.
4. **Say what happens next.** Only what's true, for example "the manager decides on any refund".

## Needs and produces
Needs: a records skill to log follow-ups and escalations.
Produces: one follow-up or escalation record per request.

## Details
- Reasons: question not covered, large group or event, complaint, change or cancellation, failed booking, other.
- Summary: at most 12 words of facts. Details: one or two sentences of what the person wants.
- A person declining a call back is a preference, never a refund promise.
- If live transfer isn't available, say so the first time a person is asked for, and offer to take a message.
