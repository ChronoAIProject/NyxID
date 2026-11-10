---
name: phone-number-sg-telnyx
description: "Get a Singapore phone number on Telnyx for a voice agent and SMS, including Singapore's requirement documents, then verify it through NyxID."
version: "1.0"
license: MIT
metadata:
  category: plain
  tag:
    - phone-number
    - singapore
    - telnyx
    - capability
---

# Singapore number on Telnyx

## Overview
I walk you through getting a Singapore phone number on Telnyx and connect it to
your Telnyx voice agent once it's active.

It suits a +65 line answered by a Telnyx AI assistant. It is not instant:
Singapore numbers need approved requirement documents first.

It needs a Telnyx account with billing. Ordering happens in the Telnyx portal;
NyxID cannot order numbers today.

## Approach
1. **Prepare the documents.** Telnyx asks for Singapore requirements, usually business registration and a local address.
   Asks you
2. **Order the number.** You search and order a +65 number in the Telnyx portal with those documents.
   Asks you
3. **Connect it to the agent.** Assign the number to the voice agent's call-control or TeXML application.
4. **Test it.** You place a test call and I check the agent answered.
   Asks you

## Needs and produces
Needs: a Telnyx account with billing, and your business registration and address documents.
Produces: an active +65 Telnyx number connected to your voice agent.

## Details
- Telnyx portal: Numbers → Search & Buy → Singapore; complete the requirement group before the order can finish.
- Assign the number to the assistant's TeXML application (the assistant's `telephony_settings.default_texml_app_id`).
- For SMS from the same number, enable a messaging profile on it.
- NyxID has no number-ordering operation on Telnyx today; don't attempt one through the generic proxy without the owner's explicit say-so.
