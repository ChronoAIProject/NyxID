# Platform protocol compatibility evidence

The reference revision is `dbda4e328967970708cd64b334b1f95b2aea24eb`. The table tracks entry-point comparisons for the 11 surfaces in [#2830]. [#2831] owns the shared replay fixtures and comparator. [#2846] tracks cross-version handoffs and rollback evidence. “Pending” means this matrix has no recorded paired run for that row.

| Entry points | Implementation dependencies | Comparison and handoff to verify | Recorded result |
| --- | --- | --- | --- |
| Channel list, proactive send and reply | [#2832] | Compare direct messages, public posts and wildcard reply targets across all three entry points. Register the test adapter with two target capabilities. Compare authorization, sends, attachment rejection and list reads. | Pending |
| Inbound channel events, text and receipts | [#2833], [#2834] | Replay duplicate events, expired claims and partially sent multipart replies. Compare claim namespace and TTL, text boundaries, accepted message IDs and final receipts when the other version resumes. | Pending |
| Channel registration and clients | [#2835] | Run the existing CLI registration and update commands with the same environment. Compare request order, omitted and cleared fields, output and exit status. Compare the web phone forms' submitted JSON. | Pending |
| LLM discovery, translation and execution | [#2836], [#2837] | Compare both service insertion orders for a provider with zero, one or two connections. Capture exact bytes from each public LLM entry for absent, false and true `include_usage`. Compare stream termination and metering. | Pending |
| Hosted OAuth and node credentials | [#2838] | Exercise both server credential stores independently using top-level and nested token responses, rate limits and late competing refreshes. Compare final credential state and the node request encoding. | Pending |
| Social login and mobile clients | [#2839] | Compare browser callback cookies, redirects and native provider exchanges. Exercise old-to-new and new-to-old handoff, MFA and unsupported mobile providers. Record observed coverage below. | Browser, mobile callbacks and mixed-version flow consumption passed in both directions. See below. |
| SMS and telephone calls | [#2840], [#2841] | Compare each carrier's authorized request, segment count and sends. Resume an existing call on the other version after restart. Replay late callbacks and compare frozen usage, reservations and settlement. | Pending |
| Realtime voice | [#2842] | Exercise SDP success and rejection, undelivered WebSocket setup, lease transfer, disconnect and Stop. Compare transport events, accepted work and settlement after a repeated usage event. | Pending |
| MCP tools, attachments and resources | [#2843] | Compare discovery and execution for second connections, custom overrides, unpublished operations, attachment writes and resource source checks. Compare schema, upstream calls, receipts and audit events. | Pending |
| HTTP proxy, credential injection and nodes | [#2844] | Exercise direct and node routes with supported and rejected protocols. Compare outbound bytes, verified signatures, node frames, receipts and settlement when a node disconnects after send. | Pending |
| Notifications and mobile push | [#2845] | Exercise ordinary and silent push, APNs topic selection, unknown devices, empty tokens and a partially successful fan-out. Compare delivery count, cleanup and persisted device state. | Pending |


## Social login evidence

On 2026-11-26 the reference server was the published Linux 0.78.0 image at
`sha256:7a46e884d2e45361f4fe6649b6587b07dba24f5227b445932cb62a6d3c8507df`.
The candidate was built from this change on `c03a6637dcfa`. Both served their
normal HTTP routes with the same disposable MongoDB replica set, callback origin,
JWT keys and local test provider configuration.

- Run `python3 scripts/test-social-login-mixed-binaries.py 29142 29141` with
  the reference and candidate listening on those loopback ports. Google,
  GitHub and Apple produced matching authorization redirects and cookie
  attributes. Browser cancellation used GET for Google and GitHub and
  form POST for Apple. Each version accepted a mobile flow created by the
  other, returned the expected cancellation URI, and refused a second
  consumption. A replay of the six mobile starts recorded six inserts and
  twelve consumption attempts in MongoDB, with zero flow records left.
- Each version returned HTTP 400 with the same provider-specific
  `invalid_request` for the wrong proof type at `POST /oauth/token`.
- For the session handoff, each version registered a synthetic user with a
  test invitation. A browser login cookie opened the same account through
  `GET /api/v1/users/me` on the other version. A mobile email login's bearer
  token also opened that account on the other version, whose `POST /api/v1/auth/refresh`
  issued a successor accepted by the original version. Both directions passed.
- On an iPhone 14 Pro simulator running iOS 26.3, an out-of-order API fixture
  containing an unknown provider produced the Google, GitHub and Apple buttons
  in that order. Each button opened its own system authentication session with
  the mobile callback URI. Cancelling returned to the sign-in screen, and a
  subsequent tap issued a new request.


## Reproducing a comparison

1. Record both Git revisions, client and dependency versions, the fixture or seed, and the exact invocation. Run through the public entry point against an isolated upstream and the same initial database snapshot. First replay the reference revision twice to establish reproducibility.
2. Compare the externally visible response, ordered upstream requests, MongoDB commands and final state, and audit, billing and receipt records. For random cryptographic fields compare their verification results, counts and associations. Record each difference and which scenarios the run covered.
3. For a handoff, let one version create the record or acquire the claim and the other resume it in the same database. Repeat from a fresh seed in reverse order. Record completion, duplicate effects and rollback results in the owning issue. Link the fixture, exact commands and evidence to the row.

[#2830]: https://github.com/ChronoAIProject/NyxID/issues/2830
[#2831]: https://github.com/ChronoAIProject/NyxID/issues/2831
[#2832]: https://github.com/ChronoAIProject/NyxID/issues/2832
[#2833]: https://github.com/ChronoAIProject/NyxID/issues/2833
[#2834]: https://github.com/ChronoAIProject/NyxID/issues/2834
[#2835]: https://github.com/ChronoAIProject/NyxID/issues/2835
[#2836]: https://github.com/ChronoAIProject/NyxID/issues/2836
[#2837]: https://github.com/ChronoAIProject/NyxID/issues/2837
[#2838]: https://github.com/ChronoAIProject/NyxID/issues/2838
[#2839]: https://github.com/ChronoAIProject/NyxID/issues/2839
[#2840]: https://github.com/ChronoAIProject/NyxID/issues/2840
[#2841]: https://github.com/ChronoAIProject/NyxID/issues/2841
[#2842]: https://github.com/ChronoAIProject/NyxID/issues/2842
[#2843]: https://github.com/ChronoAIProject/NyxID/issues/2843
[#2844]: https://github.com/ChronoAIProject/NyxID/issues/2844
[#2845]: https://github.com/ChronoAIProject/NyxID/issues/2845
[#2846]: https://github.com/ChronoAIProject/NyxID/issues/2846
