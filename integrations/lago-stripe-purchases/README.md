# NyxID external credit purchases for Lago

This AGPL-3.0-only extension targets Lago API v1.50.0 at commit
`32efd35931cca340050c2623d811b14e75d78de9`. It uses Lago's native wallet
transaction and settlement services. No Premium payment API is required.
The extension stores actual Stripe subtotal, tax, and gross in a dedicated
receipt, and settles an inbound purchased lot in the same PostgreSQL
transaction. It creates no Lago purchase invoice, payment job or customer
document. Stripe owns the customer payment and purchase document.

## Install on a pinned local instance

Copy the controller to `app/controllers/api/v1/`, the models to `app/models/`
and the migrations to `db/migrate/` in the pinned Lago API source or image.
Copy `routes.rb` to `config/routes/nyxid_external_credit_purchases.rb` and
add `draw :nyxid_external_credit_purchases` inside the main
`Rails.application.routes.draw` block. Run the migrations before starting
the updated Lago API and workers. Distribute the corresponding Lago source
and this extension under the terms in [LICENSE](LICENSE).

Create a dedicated Lago organization API key for NyxID. Supply its internal
ID as `NYXID_CREDIT_PURCHASE_API_KEY_ID` and configure a separate high-entropy
`NYXID_CREDIT_PURCHASE_SECRET` of at least 32 bytes. Set
`NYXID_CREDIT_PURCHASE_STRIPE_ACCOUNT_ID` and
`NYXID_CREDIT_PURCHASE_STRIPE_MODE` to the Stripe account ID and its `test`
or `live` mode. Keep the additional integration header out of HTTP logs.
The extension requires the exact key identity and the separate secret for
both settlement and receipt access, including on OSS installations that
disable generic resource permissions.

NyxID requires capability version 2, which includes the decision-fence migration
and the separate cancellation route. Upgrade the API and workers with the new
extension routes and run migration `20261118000002` before enabling the updated
NyxID checkout or enrolling wallets. The cancellation route either records a
fence for the original decision ID or returns its committed void receipt.

If a concurrent index build in migration `20261118000002` fails, check
`pg_index.indisvalid` and `pg_get_indexdef(indexrelid)` for the indexes on
`nyxid_external_credit_refunds` in the Lago schema. The migration creates
`index_nyxid_external_credit_refunds_on_decision_id` and
`idx_nyxid_external_applied_reversals`. Compare the definition with the migration.
With autocommit enabled, run `DROP INDEX CONCURRENTLY` for only the schema-qualified
index you confirmed belongs to this migration and has `indisvalid = false`.
Rerun `bin/rails db:migrate` and check that the authenticated wallet capability
endpoint returns HTTP 200 with `version: 2`.

NyxID checks capability version, wallet, Stripe account and mode before opening
Checkout. Receipt reads and exact replay provide recovery if a response is lost
after commit. PostgreSQL uniqueness applies to both Checkout and PaymentIntent
identity. A conflicting replay returns an error. The stock Lago payment-status
update leaves a credit invoice receivable and is unsuitable for this flow.

## Exercise the routes locally

Use a loopback Lago URL and writable replica-set MongoDB. The combined test
runs mounted NyxID HTTP routes with actual Lago wallet settlement and a Stripe
HTTP fixture. The fixture represents a USD 100 subtotal, USD 9 tax and USD 109
total for 100 credits.

```sh
export NYXID_TEST_LAGO_EXTENSION_URL=http://127.0.0.1:17408
# Set NYXID_TEST_LAGO_EXTENSION_KEY and NYXID_TEST_LAGO_EXTENSION_SECRET
# from the local connector configuration without printing them.
cargo test -p nyxid --bin nyxid-server stripe_purchase_through_billing_routes_with_local_lago -- --ignored
```

The ordinary test `stripe_purchase_through_billing_routes_recovers_paid_credits_and_late_document`
also injects a lost settlement response and tests delayed payment and invoice
publication. Use a Stripe test account and real Checkout to verify seller and
buyer fields, tax registrations, the final PDF and Portal edits.
