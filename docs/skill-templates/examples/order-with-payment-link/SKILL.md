---
name: order-with-payment-link
description: "Take an order from the business's catalog, save it, give the exact total the records calculate, and send a payment link; the order is confirmed once paid."
version: "1.0"
license: MIT
metadata:
  category: plain
  tag:
    - orders
    - payments
    - workflow
---

# Order with payment link

## Overview
I take an order from the business's catalog, save it, give the exact total the
records calculate, and send a payment link. The order is confirmed once paid.

It suits takeaway, pickup and simple product orders. I never make up prices or
promise a pickup before payment.

It needs a records skill, a payment-link skill and a messaging skill.

## Approach
1. **Help them choose.** Only catalog items with a price that aren't sold out.
2. **Collect pickup and contact.** A pickup time and a mobile number for the payment link.
3. **Read the order back once.** The items, pickup time and name in one line.
   Asks you
4. **Save the order.** I save it and read the total from the saved record.
5. **Send the payment link.** I create the link for that exact total and text it.
6. **Confirm what happens next.** The total, and that the order is confirmed once they pay.

## Needs and produces
Needs: a records skill with the catalog and orders, a payment-link skill, a messaging skill, and the business's order rules.
Produces: a saved order with its total, and a payment link sent by SMS.

## Details
- Items not in the catalog, or without a price, are not available: say so kindly and log a quiet catalog fix for staff.
- Never add up prices yourself. The only total you may say is the one the saved record returns.
- Never say they pay at pickup or that the order is confirmed before payment.
- If the payment link or text fails, say so and log a follow-up; don't create a second order.
