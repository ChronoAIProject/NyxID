---
name: phone-front-desk-sg
description: "Skillset for one use case: an AI phone front desk for a small business in Singapore that answers calls, takes bookings and paid orders, texts confirmations and logs follow-ups. Built from reusable NyxID capability skills."
kind: generic
tags:
  - phone
  - front-desk
  - singapore
  - use-case
members:
  - phone-number-sg-telnyx@1.0
  - voice-agent-telnyx@1.0
  - voice-agent-elevenlabs@1.0
  - records-notion@1.0
  - sms-send-twilio@1.0
  - payment-link-stripe@1.0
  - phone-conversation@1.0
  - phone-call-closing@1.0
  - caller-details-readback@1.0
  - confirm-from-tool-result@1.0
  - answer-from-knowledge@1.0
  - human-follow-up@1.0
  - booking-with-availability@1.0
  - order-with-payment-link@1.0
---

# Phone front desk (Singapore)

## Overview
This skillset is for one use case: an AI front desk that answers a small
business's phone in Singapore when the team can't pick up. It answers questions,
takes bookings and paid takeaway orders, texts confirmations and logs anything
staff must handle.

It suits restaurants, salons and clinics with one line and a short menu or
service list. It is not a call centre, cannot transfer calls to a person, and
does not include the business's own facts.

You need a Telnyx line, ElevenLabs for the voice, Notion for records, Twilio for
texts, Stripe for payments, and a facts pack for the business.

## Approach
1. **Get a Singapore number.** A +65 line on Telnyx, with the documents Singapore requires.
   Uses: phone-number-sg-telnyx · Asks you
2. **Create the voice agent.** The Telnyx assistant that answers every call.
   Uses: voice-agent-telnyx
3. **Choose the voice.** A natural ElevenLabs voice for your callers' languages.
   Uses: voice-agent-elevenlabs · Asks you
4. **Set up the records.** Notion databases for bookings, orders, follow-ups and availability.
   Uses: records-notion
5. **Turn on text confirmations.** Booking confirmations and payment links by SMS.
   Uses: sms-send-twilio
6. **Turn on payment links.** Stripe links for takeaway orders, confirmed once paid.
   Uses: payment-link-stripe
7. **Run test calls.** You call the line and we check every flow end to end before going live.
   Asks you

## Needs and produces
Needs: Telnyx, ElevenLabs, Notion, Twilio and Stripe connections in NyxID, plus a business facts pack.
Produces: a live +65 phone front desk, with every booking, order and follow-up saved in Notion.

## Details

### On every call
The voice agent follows these member skills:
1. phone-conversation for every reply, and answer-from-knowledge for questions.
2. caller-details-readback before saving anything.
3. booking-with-availability for bookings, and order-with-payment-link for takeaway.
4. confirm-from-tool-result for every claim that something is done.
5. human-follow-up for anything staff must handle.
6. phone-call-closing to end the call.

### Roles
- Records: records-notion. Messaging: sms-send-twilio. Payments: payment-link-stripe.
- Phone line: phone-number-sg-telnyx with voice-agent-telnyx; the voice comes from voice-agent-elevenlabs.

### Swapping a provider
Publish a new revision with a different member for that role, for example
sms-send-telnyx instead of sms-send-twilio, or records-google-sheets instead of
records-notion. The workflow and behaviour skills stay the same.

### Not included
The business's facts (hours, rules, FAQ, catalog) live in a separate facts pack
attached beside this skillset. Live call transfer is not available.
