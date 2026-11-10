---
name: voice-agent-telnyx
description: "Create and maintain a Telnyx AI voice assistant through NyxID: instructions, greeting, model and voice, answering calls on a Telnyx number."
version: "1.0"
license: MIT
metadata:
  category: tool-based
  tool-list:
    - list_assistants
    - get_assistant
    - create_assistant
    - update_assistant_instructions
    - list_voices
  tag:
    - voice-agent
    - telnyx
    - capability
---

# Voice agent on Telnyx

## Overview
I set up a Telnyx AI assistant that answers your phone line: its instructions,
greeting, model and voice. I can also update its instructions later.

It suits a phone agent that must answer real calls on a Telnyx number. Tools the
agent calls during a call, such as bookings and orders, are wired in the Telnyx
portal.

It needs a Telnyx connection in NyxID. Calls are billed by Telnyx.

## Approach
1. **Find or create the assistant.** I look for an existing assistant before creating one.
2. **Write its instructions.** The behaviour and facts the agent follows on every call, plus its greeting.
3. **Choose model and voice.** A fast model and a voice that speaks your callers' languages.
   Asks you
4. **Check it.** I read the assistant back and confirm the instructions and voice took effect.

## Needs and produces
Needs: a Telnyx connection in NyxID; a Telnyx number for real calls.
Produces: a Telnyx assistant id with the agreed instructions, greeting and voice.

## Details
- `list_assistants` / `get_assistant` before `create_assistant`; never create duplicates.
- Instructions replace wholesale: use `update_assistant_instructions` if your connection offers it, otherwise edit in the Telnyx portal. Read back with `get_assistant`.
- ElevenLabs voices are referenced as `ElevenLabs.<model>.<voice_id>` in `voice_settings.voice`, with the ElevenLabs key stored in Telnyx.
- Telnyx inserts variables such as the current time and caller id at call time.
- Tools the agent calls during a call (webhooks) are configured in Telnyx, not through these operations.
