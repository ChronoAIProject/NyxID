---
name: voice-agent-elevenlabs
description: "Choose voices and create ElevenLabs conversational agents through NyxID, for heavily customised voices and web or phone voice agents."
version: "1.0"
license: MIT
metadata:
  category: tool-based
  tool-list:
    - list_voices
    - list_models
    - create_convai_agent
    - get_convai_agent
    - list_convai_agents
    - text_to_speech
  tag:
    - voice-agent
    - elevenlabs
    - capability
---

# Voice agent on ElevenLabs

## Overview
I help you pick an ElevenLabs voice and create an ElevenLabs conversational
agent with its prompt, first message and voice. I play voice samples, and you
try the agent from its page in the ElevenLabs dashboard.

It suits heavily customised voices and agents. Changing an existing agent is not
available through NyxID yet; that happens in the ElevenLabs dashboard.

It needs an ElevenLabs connection in NyxID. Usage is billed by ElevenLabs.

## Approach
1. **Pick a voice.** I list voices that suit your languages and play a short sample.
   Asks you
2. **Create the agent.** Its prompt, first message, language and chosen voice.
3. **Try it.** You talk to the agent with the test button on its ElevenLabs dashboard page, then tell me what to change.
   Asks you
4. **Hand it to the phone line.** I give the voice and agent ids to the phone provider setup.

## Needs and produces
Needs: an ElevenLabs connection in NyxID.
Produces: a chosen voice id and an ElevenLabs agent id, tried in the dashboard.

## Details
- `list_voices` to choose; preview with `text_to_speech` using a short sentence in each language the agent will speak.
- Prefer a multilingual model (for example `eleven_multilingual_v2`) when callers mix languages.
- `create_convai_agent` with `conversation_config.agent.prompt.prompt`, `first_message` and `language`, plus `tts.voice_id`.
- To try the agent, send the person to its page in the ElevenLabs dashboard. `get_convai_signed_url` returns a short-lived WebSocket URL for an app built on the ElevenLabs SDK, not a page a person can open, so never give it out as a test link.
- Updating an agent has no NyxID operation yet: change it in the ElevenLabs dashboard, then confirm with `get_convai_agent`.
