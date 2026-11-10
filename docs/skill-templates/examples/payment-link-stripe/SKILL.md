---
name: payment-link-stripe
description: "Create a Stripe payment link or checkout session for one exact amount through NyxID, check whether it was paid, and expire it when an order is cancelled."
version: "1.0"
license: MIT
metadata:
  category: tool-based
  tool-list:
    - create_price
    - create_payment_link
    - create_checkout_session
    - retrieve_checkout_session
    - expire_checkout_session
    - retrieve_payment_intent
  tag:
    - payments
    - stripe
    - capability
---

# Payment link with Stripe

## Overview
I create a Stripe payment page for one exact amount, give you the link to send,
and later check whether it was paid or expire it if the order is cancelled.

It suits paying for one order, deposit or invoice by link. It is not for
subscriptions or saving cards.

It needs a Stripe connection in NyxID that can create prices and links; the
shared Stripe catalog entry is read-only today. Stripe charges its usual fees.

## Approach
1. **Set the amount.** I take the exact amount and currency from the saved order, never my own arithmetic.
2. **Create the payment page.** A one-off price, then a payment link or checkout session for it.
3. **Hand over the link.** I return the URL for you or a messaging skill to send.
4. **Check or cancel.** I check whether it was paid, or expire it when the order is cancelled.

## Needs and produces
Needs: a Stripe connection in NyxID with write access to prices, payment links and checkout sessions.
Produces: a payment URL with its Stripe id, and later a paid, unpaid or expired status.

## Details
- Amounts are integers in the smallest currency unit (SGD 37.90 is `unit_amount=3790`). Take them from the saved order total.
- Price: `POST /v1/prices` with `currency`, `unit_amount`, `product_data[name]`.
- Checkout session: `POST /v1/checkout/sessions` with `mode=payment`, `line_items[0][price]`, `line_items[0][quantity]=1`, `success_url`, and `expires_at` (30 minutes to 24 hours ahead). Add `payment_method_types[]=paynow` for Singapore PayNow where enabled.
- Payment link (no expiry of its own): `POST /v1/payment_links`; deactivate it later with `active=false`.
- Paid means `payment_status=paid` on the session (or a succeeded PaymentIntent). The receipt is `latest_charge.receipt_url` on the PaymentIntent.
- Cancel an unpaid session with `POST /v1/checkout/sessions/{id}/expire`.
- Use test-mode keys until the business is ready; never print secret keys or card data.
