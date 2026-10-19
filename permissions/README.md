# Google API permissions and Drive folder proof of concept

A reusable Rust permission evaluator and a local REST/MCP proxy. The proxy can
run against an in-memory Drive or an existing NyxID Google connection. It is
experimental. Native permission-bound keys now use the same evaluator through
dedicated backend endpoints; see [the native pilot guide](../docs/NATIVE_DRIVE_PERMISSIONS.md).
Existing general-purpose keys retain their ordinary authority.

The native host now also accepts [declarative Google REST policies](../docs/GOOGLE_API_PERMISSIONS.md).
Operation definitions pin an API origin, method, resource IDs, query rules,
and a closed JSON body schema. Examples cover Workspace, Cloud, YouTube, and
Gemini. The standalone executable and its ordinary-proxy transport remain
Drive-only; generic transports must attest to the final Google API origin.

Run the executable smoke test from the repository root:

```sh
python3 scripts/test-drive-permissions-poc.py
```

See [the POC guide](../docs/DRIVE_PERMISSIONS_POC.md) for the boundary, supported
operations, live configuration, extension points, and tests. Edit
[the example policy](examples/drive-policy.json) to experiment with constraints.
[The hook example](examples/drive-hooks-policy.json) adds a live write-pause
switch and a response marker check, enforced by the same engine over REST/MCP.
