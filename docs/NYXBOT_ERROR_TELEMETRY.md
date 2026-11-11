# Nyxbot error events

Nyxbot reuses NyxID's existing authenticated analytics relay and backend PostHog
exporter. The browser captures the custom `nyxbot.error` event; the server
allowlist validates its properties before export. This change adds the event to
both schemas. No new vendor, SDK, environment variable, or billing change is
required. Deploy both backend and frontend together so the relay accepts it.

## Coverage

The native NyxAgent, Aevatar, and direct model transports report unsuccessful
HTTP requests (including 403 and 5xx), network failures, timeouts, and failed
turns delivered inside HTTP 200 streams. Assistant history, state, management,
and catalog requests through `assistantHttp` report HTTP/network failures;
`assistantJson` also reports invalid JSON responses.

Image/file uploads through the native attachment transport report unsuccessful
HTTP responses, connection failures, upload timeouts, and invalid successful
responses. Local file-size validation and cancelled uploads are excluded.

A catch-all on `/assistant` and its subpages records unexpected JavaScript
errors, unhandled promise rejections, and failed image/script/stylesheet loads.
Assistant route error boundaries also report caught rendering/loading errors.
Resource events deliberately omit the resource URL; browsers do not expose its
HTTP status in a load-error event. They therefore cannot distinguish an image
403 from an image 500. The catch-all excludes authentication/link pages.

Each active send reports at most one terminal error. Errors that wrap an already
reported error are deduplicated through their cause chain, so route/window
fallbacks do not recount the same error object. Separate failed history or
management requests produce separate events. User stops, cancellation, identity
teardown, successful turns, and best-effort stop cleanup are excluded. Reloading
stored failed messages does not count old failures again.

This is browser telemetry: autonomous channel turns that never reach the browser
remain outside this feature. Backend logs remain necessary for server/provider
diagnostics, non-consenting users, browser shutdown, and unavailable networks.
No frontend catch-all can guarantee delivery of every error.

## Event contract

| `error_code` | Meaning |
| --- | --- |
| `NYXBOT_HTTP_UNAUTHORIZED` | HTTP 401 |
| `NYXBOT_HTTP_FORBIDDEN` | HTTP 403 |
| `NYXBOT_HTTP_RATE_LIMITED` | HTTP 429 |
| `NYXBOT_HTTP_SERVER_ERROR` | HTTP 5xx |
| `NYXBOT_HTTP_ERROR` | Other unsuccessful HTTP response |
| `NYXBOT_RUN_ERROR` | Explicit failed turn, including inside HTTP 200 |
| `NYXBOT_START_TIMEOUT` | Reply did not start, or an upload did not finish, before the deadline |
| `NYXBOT_STREAM_TIMEOUT` | Stream stopped making progress |
| `NYXBOT_PROTOCOL_ERROR` | Invalid JSON or missing/invalid stream protocol data |
| `NYXBOT_STREAM_ERROR` | Stream read failed |
| `NYXBOT_NETWORK_ERROR` | Request failed before a response arrived |
| `NYXBOT_RESOURCE_ERROR` | Image, script, or stylesheet failed to load |
| `NYXBOT_UNKNOWN_ERROR` | Unexpected error caught by a fallback |

These are analytics strings; existing numeric backend `AppError` codes retain
their meaning. The relay accepts bounded numeric `upstream_error_code` values
and a finite list of native error codes. Unknown upstream text is dropped while
the generic event code remains countable.

The browser submits at most twelve properties: `error_code`, `engine`, `stage`,
`route`, `http_status`, `upstream_error_code`, `request_id`, `correlation_id`,
`trace_id`, `conversation_id`, `turn_id`, and `duration_ms`. Optional properties
are omitted when unavailable. Engines are `nyxagent`, `aevatar`, `direct`, or
`unknown`; stages are `request`, `stream`, `runtime`, or `resource`. Routes use
fixed templates such as `/assistant/conversations/:id/state`. NyxID adds its
existing common properties (surface, environment, and version) on the backend.

The relay validates UUID correlation identifiers (including supported native
conversation/turn prefixes) and replaces them with SHA-256 hashes named
`request_id_hash`, `correlation_id_hash`, `trace_id_hash`,
`conversation_id_hash`, and `turn_id_hash`. Invalid identifiers are dropped.
The hash input is the full original identifier, lowercased, including any
prefix. Raw identifiers do not leave the backend through this event. Trace
metadata uses `X-NyxID-Trace-Id`, with the other existing trace/request headers
and an explicitly labeled trace UUID as fallbacks where supported.

Prompts, replies, raw error messages, stack traces, response bodies, arbitrary
headers, credentials, resource URLs, and query parameters are excluded. The
existing account consent, local restriction, Do Not Track, account-generation
guards, bounded relay concurrency, and backend scrubbing remain in force.
Request and upload reporting also checks the originating account revision, so
late errors are excluded after switching accounts, even when switching back.
Telemetry failures never change the chat's settlement or displayed error.

## Viewing errors in PostHog

Use the existing NyxID PostHog project. Create a Trends insight for
`nyxbot.error` and break down by `error_code`. Use total events for failure volume
or unique users for affected users. Filter by `http_status`, `engine`, `stage`,
or `route` to narrow the source. These are custom analytics events, so they
appear in event/insight views rather than PostHog exception issues.

```sql
SELECT
    properties.error_code AS error_code,
    count() AS failures,
    uniq(distinct_id) AS affected_users
FROM events
WHERE event = 'nyxbot.error'
  AND timestamp >= now() - INTERVAL 7 DAY
GROUP BY properties.error_code
ORDER BY failures DESC
```

To correlate a support report, calculate SHA-256 of the lowercased displayed
trace UUID, then filter `properties.trace_id_hash` by that digest. For example,
this uses a dummy trace, not production data:

```python
import hashlib
trace = "76dd13d5-e7cf-4c63-a33b-26036d2a495d"
print(hashlib.sha256(trace.lower().encode()).hexdigest())
```

Counts measure observed failures. A failure rate needs matching attempt/success
events with the same coverage and consent boundaries; this feature does not
introduce that denominator.

## Deployment verification and billing

Read-only queries through the connected PostHog project work, but that does not
prove ingestion is currently enabled or that the outstanding balance permits
new events. This PR does not change PostHog billing, quota, or settings, and does
not promise zero incremental cost: these events count toward existing usage.

After deployment, use an opted-in test account to trigger a controlled 403 or
failed turn, record its server trace, and query for its `trace_id_hash` and
`nyxbot.error`. Confirm the new row's timestamp and code. An accepted capture
HTTP response alone does not prove storage when quota or billing restrictions
apply. Until that query succeeds, treat production ingestion as unverified.
