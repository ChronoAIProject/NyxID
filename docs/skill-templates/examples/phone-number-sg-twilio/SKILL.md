---
name: phone-number-sg-twilio
description: "Get a Singapore phone number on Twilio for voice and SMS, including the regulatory documents Singapore requires, then verify it works through NyxID."
version: "1.0"
license: MIT
metadata:
  category: plain
  tag:
    - phone-number
    - singapore
    - twilio
    - capability
---

# Singapore number on Twilio

## Overview
I walk you through getting a Singapore phone number on Twilio and check it works
for calls and texts once it's active.

It suits a business that wants a local +65 line for a voice agent or SMS. It is
not instant: Singapore numbers need approved business documents first.

It needs a Twilio account with billing. Buying the number happens in Twilio's
console; NyxID cannot order numbers today.

## Approach
1. **Prepare the documents.** Singapore requires a regulatory bundle, usually the business registration and a local address.
   Asks you
2. **Request the number.** You choose a mobile or local number in Twilio's console and submit the bundle.
   Asks you
3. **Point it at the agent.** Once approved, set the voice and messaging webhooks to your voice agent and SMS setup.
4. **Test it.** I send a test text and you place a test call to confirm both directions work.
   Asks you

## Needs and produces
Needs: a Twilio account with billing, and your business registration and address documents.
Produces: an active +65 number configured for voice and SMS.

## Details
- Twilio Console: Phone Numbers → Regulatory Compliance → create a Singapore bundle (end-user type Business), then buy the number against it. Approval can take days.
- Singapore mobile numbers start +65 8 or +65 9; landlines (+65 6) cannot receive SMS.
- Record the number in E.164 format; SMS skills use it as `From`.
- NyxID has no number-search or purchase operation on Twilio today; don't attempt one through the generic proxy without the owner's explicit say-so.
