---
name: sms-send-telnyx
description: "Send an SMS through a Telnyx connection in NyxID and check its delivery status. Use for confirmations, reminders and links when the number is on Telnyx."
version: "1.0"
license: MIT
metadata:
  category: tool-based
  tool-list:
    - send_message
    - get_message
  tag:
    - sms
    - telnyx
    - capability
---

# Send SMS with Telnyx

## Overview
I send a text message from your Telnyx number to one phone number, then check
its delivery status.

It suits confirmations, reminders and links when your number lives on Telnyx.
It is not for bulk campaigns.

It needs a Telnyx connection in NyxID and a messaging-enabled Telnyx number.
Each message is billed by Telnyx.

## Approach
1. **Check the number.** I make sure the recipient is a mobile number in international format.
2. **Write the text.** One short message with only what the person needs.
3. **Send it.** I send it from your messaging-enabled Telnyx number.
4. **Check delivery.** I read the message status and report sent, delivered or failed.

## Needs and produces
Needs: a Telnyx connection in NyxID and a messaging-enabled Telnyx number.
Produces: one SMS, with its Telnyx message id and delivery status.

## Details
- Send with `send_message`: `from` (your Telnyx number, E.164), `to` (E.164), `text`.
- Success is a response with `data.id` and a `to[0].status` such as `queued` or `sent`. An error or missing id means it was not sent.
- Check later with `get_message` by id: `delivered`, `delivery_failed` or `sending_failed`.
- No secrets in texts, one message per event, and never retry an unknown result.
