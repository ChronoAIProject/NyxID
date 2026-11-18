These fixtures were captured on 2026-10-29 from the actual Codex app-server
0.164.0 (locally installed) and latest release rust-v0.165.0 (published 2026-10-27).
NyxAgent f6988c5a3955f69cfd895fcfa7c9e5c2a986541f configures runtime.channel=latest;
there is no checked-in production pin. We cannot infer the serving runtime
version from a NyxID health SHA.

Each isolated process used NyxAgent's custom model catalog, feature overrides,
initialize/thread/start/turn/start RPCs and all six dynamic function tools from
src/nyxid.rs. Inputs and instructions were synthetic. Host skill/environment
text was replaced with a fixed placeholder; structure and runtime correlation
fields were retained. A local HTTP server captured Responses. The fixtures
apply the bridge's exact model/store rewrites and function/web_search filter.
Chat and research enable web_search; channel disables it. No production prompt,
credential or guest content was captured.

The bridge does not forward Codex's inbound headers. Its reqwest 0.13.5
ClientBuilder uses bearer_auth().json(), producing Authorization (synthetic in
tests), Content-Type: application/json, Accept: */*, Host and Content-Length.
NyxID strips authorization/host/content-length before semantic identity. Reqwest
features resolved from NyxAgent Cargo.lock are json/stream/rustls/http2; no
compression or user-agent default is enabled. Contract tests use these exact
semantic headers, not Codex's incoming tracing headers.

All six captured bodies pass the original guest shape allowlist. In contrast,
the original owner classifier explicitly rejects client_metadata. Do not cite
an unknown top-level field as a production cause without a recorded diagnostic.
When NyxAgent's profile mapping or Codex runtime contract changes, recapture and
review these fixtures and model-routes.json together. Unknown extensions fail
closed and emit bounded, metadata-only diagnostics.
