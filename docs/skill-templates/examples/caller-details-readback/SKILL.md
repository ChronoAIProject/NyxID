---
name: caller-details-readback
description: "Collect the details a booking, order or message needs without echoing each one, then read everything back once before saving, offering the caller's own number by its last four digits."
version: "1.0"
license: MIT
metadata:
  category: plain
  tag:
    - phone
    - voice
    - behaviour
---

# Caller details read-back

## Overview
I collect the details a booking, order or message needs without repeating each
one back, then read everything back once, in one quick line, before anything is
saved.

It suits any phone flow that saves something for the caller, who hears their
details once and can correct them before it counts.

It needs the caller ID. It changes nothing by itself.

## Approach
1. **Ask for what's missing.** One question at a time, a short "okay" after each answer, and no echo.
2. **Offer the caller's own number.** From the caller ID, by its last four digits, inside the read-back.
3. **Read back once.** One line with the key details and the number question, then I wait for yes.
   Asks you
4. **Act on yes.** When the caller says yes, the next step saves it straight away.

## Needs and produces
Needs: the caller ID, and which numbers count as mobiles in your region.
Produces: details the caller has confirmed, ready to save.

## Details
- Never echo a detail back as a question while collecting. Say "okay" and ask for the next missing detail.
- One read-back with everything, including the date as a real date ("Saturday the seventh"). Read back again only if something changes.
- The read-back is its own reply with no tool call, and never says anything is already saved.
- Caller ID: if it is a real number (not withheld, not the business's own line), offer it by its last four digits, copied digit by digit. Once agreed, never say it again; reuse it silently in full international format.
- Texts need a mobile: if the number can't receive SMS, ask for a mobile, or carry on without the text.
- Always ask for a name and use it exactly as said.
