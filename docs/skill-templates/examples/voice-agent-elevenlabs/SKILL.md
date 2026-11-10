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
    - get_convai_signed_url
    - text_to_speech
  tag:
    - voice-agent
    - elevenlabs
    - capability
---

# Voice agent on ElevenLabs

## Overview
I help you pick an ElevenLabs voice and create an ElevenLabs conversational
agent with its prompt, first message and voice. I can preview the voice and give
you a web link to try the agent.

It suits heavily customised voices and agents. Changing an existing agent is not
available through NyxID yet; that happens in the ElevenLabs dashboard.

It needs an ElevenLabs connection in NyxID. Usage is billed by ElevenLabs.

## Approach
1. **Pick a voice.** I list voices that suit your languages and play a short sample.
   Asks you
2. **Create the agent.** Its prompt, first message, language and chosen voice.
3. **Try it.** I give you a signed web link to talk to the agent before it takes calls.
   Asks you
4. **Hand it to the phone line.** I give the voice and agent ids to the phone provider setup.

## Needs and produces
Needs: an ElevenLabs connection in NyxID.
Produces: a chosen voice id and an ElevenLabs agent id with a test link.

## Details
- `list_voices` to choose; preview with `text_to_speech` using a short sentence in each language the agent will speak.
- Prefer a multilingual model (for example `eleven_multilingual_v2`) when callers mix languages.
- `create_convai_agent` with `conversation_config.agent.prompt.prompt`, `first_message` and `language`, plus `tts.voice_id`.
- `get_convai_signed_url` gives a short-lived web test link; never post it publicly.
- Updating an agent has no NyxID operation yet: change it in the ElevenLabs dashboard, then confirm with `get_convai_agent`.
