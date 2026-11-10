---
name: booking-with-availability
description: "Take a booking or appointment during a conversation: work out the exact date, check real availability in the records, read back once, save, and send a confirmation."
version: "1.0"
license: MIT
metadata:
  category: plain
  tag:
    - bookings
    - workflow
---

# Booking with availability

## Overview
I take a booking or appointment: I work out the exact date, check real
availability, read it back once, save it, and send a confirmation.

It suits restaurants, salons, clinics and anyone with slots. Groups above the
business's limit get a follow-up instead of a booking.

It needs a records skill with availability and bookings, and a messaging skill
for the confirmation.

## Approach
1. **Work out the date and time.** I turn "tomorrow" or "Saturday" into a calendar date from today's date.
2. **Check availability.** I check the exact slot in the records before reading anything back.
3. **Collect the rest.** Party size or service, and a name.
4. **Read it back once.** Date, time, size and name in one line.
   Asks you
5. **Save and confirm.** I save the booking, then send the confirmation with its reference.
6. **Handle changes.** A change after saving becomes a change request, never a second booking.

## Needs and produces
Needs: a records skill with availability and bookings, a messaging skill, and the business's booking rules.
Produces: a saved booking with its reference, and a confirmation sent.

## Details
- "Later", "tonight" and "this afternoon" mean today. With no day given, assume today if the time is still ahead and today is open; otherwise say so and offer the next open day.
- Never judge availability yourself: only a slot the records show as open, inside the booking windows and not in the past, is bookable. If it's full, offer at most two nearest options.
- Take the weekday from the records or a calendar, never from memory.
- Save only after the read-back yes; read the reference from the saved record and say it character by character.
- One booking per conversation; changes and cancellations become follow-ups for staff unless the records allow direct edits.
