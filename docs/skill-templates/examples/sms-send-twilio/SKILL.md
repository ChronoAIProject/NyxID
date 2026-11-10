---
name: sms-send-twilio
description: "Send an SMS through a Twilio connection in NyxID and check whether it was delivered. Use for confirmations, reminders and payment or booking links."
version: "1.0"
license: MIT
metadata:
  category: tool-based
  tool-list:
    - create_message
    - get_message
    - list_messages
  tag:
    - sms
    - twilio
    - capability
---

# Send SMS with Twilio

## Overview
I send a text message from your Twilio number to one phone number, then check
whether Twilio accepted and delivered it.

It suits confirmations, reminders and links sent after a call or an order. It
is not for marketing blasts or two-way chat threads.

It needs a Twilio connection in NyxID and a Twilio number or messaging service.
Each message is billed by Twilio.

## Approach
1. **Check the number.** I make sure the recipient is a mobile number in international format.
2. **Write the text.** One short message with only what the person needs, under 160 characters when possible.
3. **Send it.** I send it from your Twilio number or messaging service.
4. **Check delivery.** I read the message status and report sent, delivered or failed.

## Needs and produces
Needs: a Twilio connection in NyxID, the Account SID, and a sending number or messaging service.
Produces: one SMS, with its Twilio message SID and delivery status.

## Details
- Send with `create_message`: path `AccountSid`, form-encoded body `To=<E.164>&From=<your number>&Body=<text>` (or `MessagingServiceSid=` instead of `From`).
- Success is a response with a `sid` starting `SM` and `status` of `queued`, `accepted` or `sent`. Anything else, or an error, means it was not sent: say so, never "it's on its way".
- Check later with `get_message` (`delivered`, `undelivered`, `failed`). Error code 21610 means the person replied STOP: do not retry.
- Never put passwords, full card numbers or one-time codes for other services in a text. Links are fine.
- One message per event, and never retry a send whose result is unknown.
