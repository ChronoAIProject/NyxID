# Stripe managed OAuth service

NyxID seeds the `stripe` provider and `api-stripe` catalog service. Users connect
through Stripe Apps OAuth; NyxID stores access and rotating refresh tokens under
envelope encryption and injects the connected account's bearer token into Stripe
API requests. This is a business-service connector available to agents, not an
inference provider, and is separate from NyxID's Lago payment-provider setup.

## Register the managed app

1. Create a Stripe App using `stripe apps create`.
2. In the generated `stripe-app.json`, set `stripe_api_access_type` to `oauth`,
   `distribution_type` to `public`, and `allowed_redirect_uris` to include
   `https://YOUR_NYXID_HOST/api/v1/providers/callback`.
3. Add these read permissions with a purpose explaining each to users:
   `customer_read`, `invoice_read`, `payment_intent_read`, `subscription_read`.
4. Upload the app and use Stripe's external testing flow first. Submit it for
   review and publish before offering the public install link.
5. In NyxID's admin provider settings, configure the Stripe App OAuth client ID
   and put the **app developer account's secret API key** in the client-secret
   field. The authorization URL defaults to
   `https://marketplace.stripe.com/oauth/v2/authorize`; for external tests use
   the appropriate link from Stripe's dashboard. Store the link's endpoint
   without its query string, put its client ID in the client-ID field, and put any
   non-reserved mode parameters in `extra_auth_params`. NyxID generates its own
   state and redirect parameters. Configure matching test/live
   credentials and authorization URL together; this connector uses one mode per
   provider configuration, without automatic mode switching.
6. Connect `api-stripe` from NyxID and exercise callback, refresh, reconnect,
   and disconnect with a real test account before enabling it broadly.

The provider defaults to `credential_mode=both`: an operator-owned app supports
one-click connections; users can alternatively supply their own Stripe App client
ID and developer API key. Use `admin` mode to offer only the managed app.
Never put the developer API key in the service's downstream credential field.

## OAuth contract

Permissions live in the Stripe App manifest. There is no scope picker and
additional OAuth scopes are rejected. Stripe's documented install flow uses
state-based CSRF protection and does not document PKCE, so this provider does not
send PKCE parameters. Token exchange and refresh use form-encoded requests to
`https://api.stripe.com/v1/oauth/token`, with HTTP Basic authentication whose
username is the developer API key and whose password is empty. The app client ID
is used only in the authorization URL. An absent `expires_in` defaults to Stripe's
documented one-hour access-token lifetime; explicit lifetimes are respected.
Rotated refresh tokens replace the encrypted previous value.

No remote revocation endpoint is configured. Disconnect deletes/revokes local
credentials; users must uninstall the Stripe App in Stripe to remove upstream
access. Hosted OAuth is the supported setup; node-local OAuth does not implement
this provider-specific token authentication contract.

## Agent operations

The hosted `stripe` OpenAPI overlay supplies eight read operations: list and
retrieve customers, invoices, payment intents, and subscriptions. Lists support
bounded page sizes and cursor pagination; invoice/subscription status filters and
created-time filters are included. To investigate failed payments, inspect the
payment intent status and `last_payment_error` returned by Stripe.

The overlay introduces no refund, payment creation, or subscription mutation tools.
It is a discovery surface, not a proxy authorization boundary: keep the managed
app manifest read-only and use NyxID grants/operation policies to limit agents.
The connector does not inject a Stripe-Version header. Configure versioning in
Stripe or supply an explicit request header when a specific version is needed.

## References

- [Stripe Apps OAuth](https://docs.stripe.com/stripe-apps/api-authentication/oauth)
- [Stripe App permissions](https://docs.stripe.com/stripe-apps/reference/permissions)
- [Managed OAuth connectors](MANAGED_OAUTH_CONNECTORS.md)

Local mock-server tests exercise managed and BYO callback/refresh authentication,
refresh rotation, expiry without `expires_in`, and both credential stores. They do
not establish that Stripe has approved or published a deployment's app.

Run the local checks with a writable MongoDB instance (the tests create isolated
databases):

```sh
export NYXID_TEST_DATABASE_URL='mongodb://127.0.0.1:27017/?replicaSet=rs0'
cargo test -p nyxid --bin nyxid-server services::oauth_flow::tests
cargo test -p nyxid --bin nyxid-server cloud_oauth_connect_and_refresh_use_seeded_protocols
cargo test -p nyxid --bin nyxid-server oauth_refresh_contracts_cover_both_stores_and_legacy_encodings
cargo test -p nyxid --bin nyxid-server stripe_seed_uses_stripe_apps_oauth_and_curated_read_operations
cargo test -p nyxid --bin nyxid-server services::catalog_spec_registry::tests
```

Adjust the MongoDB URL for your development instance.
