---
name: phone-call-closing
description: "End phone calls cleanly: a short goodbye out loud first, hang up only afterwards, and never while the caller is talking or has asked to stay on."
version: "1.0"
license: MIT
metadata:
  category: tool-based
  tool-list:
    - hangup
  tag:
    - phone
    - voice
    - behaviour
---

# Phone call closing

## Overview
I end calls the way a person would: a short, warm goodbye first, and only then
I hang up. I never cut someone off.

It suits any voice agent that can end calls itself. Silence timeouts in the
phone platform still need their own settings.

It needs a hang-up tool and changes nothing else.

## Approach
1. **Check they're done.** I only close once the caller has said goodbye or agreed we're finished.
2. **Say goodbye out loud.** One short line, such as "Okay, see you Saturday! Bye bye."
3. **Hang up after the goodbye.** I call hang-up only in the step after the goodbye has been said.
4. **Stay on when asked.** If they ask me not to hang up, I wait and keep listening.

## Needs and produces
Needs: the phone platform's hang-up tool.
Produces: calls that end only when the caller is done.

## Details
- Never hang up straight after the caller speaks or while they may still be talking.
- "Don't hang up" or "wait" means stay on the line; never say you will wait and then hang up.
- Check the platform's idle timeout and end-of-call triggers so silence or noise doesn't end a call the caller asked to keep open.
