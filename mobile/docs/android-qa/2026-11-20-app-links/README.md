# HTTPS business-link readiness: #2706

Base: `upstream/main` at `b6d194f99`. Checked on 2026-11-20.

## Source fix and existing work

Main always supplies only API URLs to Expo's native-link generator. The fix
ports the host-selection portion of open [PR #2562](https://github.com/ChronoAIProject/NyxID/pull/2562),
which targets `dev`, so main can build native artifacts that also claim the two
configured frontend hosts. The relevant upstream commits are:

- `875312e19d2b160ab65d291528bf2f1b5cd1585e`: build host selector.
- `c604a1ce851af8723459a64bd48e424909239984`: explicit frontend requirement.
- PR head inspected: `d6b4c59ce033eb51c71734397e469ac934b30c16`.

Only the host resolver, Expo wiring, related tests and configuration docs are
ported; the source PR's SSH, OAuth, push and browser changes are excluded. This
patch additionally requires the **active** profile's frontend URL, so selecting
`api+frontend` cannot borrow the other profile's host through ordinary URL
fallback. The compatibility default remains `api`. Android's separate HTTPS
opt-in remains unchanged. Tests execute the actual Expo config function and
verify `/activity` in the generated iOS/Android host configurations.

## Build inputs and distributed artifacts

The primary checkout has a local `.env.local` with both DEV and PROD frontend
URLs configured. Neither profile currently defines `*_NATIVE_LINK_HOSTS` or
`*_ANDROID_HTTPS_APP_LINKS`: both therefore resolve to `api` and `false`. No local
environment file was changed. This is a local build-input observation, not an
inspection of remotely deployed secrets or release infrastructure.

The supplied Android QA 1.0.4 (3) APK identifies `dev.nyxid.nativecheck`, signed
by an **Android Debug** certificate:

`FA:C6:17:45:DC:09:03:78:6F:B9:ED:E6:2A:96:2B:39:9F:73:48:F0:BB:6F:89:9B:83:32:66:75:91:03:3B:9C`

Its manifest claims `nyxid.ai` broadly but omits `nyx.chrono-ai.fun`. This QA
identity must not be added to production Asset Links to manufacture verification.
Production publication needs the actual Play App Signing identity for
`fun.chronoai.nyxid` (or the approved identity of a directly distributed release).
The existing backend already serves Digital Asset Links from
`ANDROID_APP_PACKAGE` and `ANDROID_APP_SHA256_CERT_FINGERPRINTS`; the latter is a
comma-separated list of colon-delimited SHA-256 certificates. Set those existing
variables for the approved release identity on every serving environment and
expose the endpoint on each selected host. No new backend storage or endpoint is
required. See [the exact environment contract](../../../../docs/ENV.md#android-app-links).

Signed entitlements were extracted from the existing iOS artifacts, without
installing or rebuilding them:

| Artifact | Apple identity | Associated hosts |
| --- | --- | --- |
| 1.0.4 (60), reported in issue | `56DKXVFJS2.fun.chrono-ai.nyxid` | `nyx-api.chrono-ai.fun`, `api.nyxid.ai`, `nyxid.onelink.me` |
| 1.0.4 (61), supplied newer QA IPA | same identity | `api.nyxid.ai`, `nyxid.ai` |

Build 61 already claims the new frontend, while build 60 claims neither business
frontend. Neither artifact claims the old frontend. Entitlements are recorded
in [ios-artifact-identities.json](ios-artifact-identities.json); this inspection
does not establish iOS OS-association or cross-domain tap success.

## Public hosting and normal OS dispatch

Public probes returned **404** for `/.well-known/assetlinks.json` on all four
hosts: `nyxid.ai`, `nyx.chrono-ai.fun`, `api.nyxid.ai`, and
`nyx-api.chrono-ai.fun`. Frontend AASA endpoints returned **200**, naming
`56DKXVFJS2.fun.chrono-ai.nyxid` and including `/activity`. The sanitized responses
are in [public-associations.json](public-associations.json).

On the isolated Android API 34 emulator, the supplied QA APK was installed and
OS verification was requested with `pm verify-app-links --re-verify`. The
`nyxid.ai` state remained `legacy_failure`. Normal, unscoped VIEW dispatch of
both HTTPS `/activity` URLs opened the system WebView browser. A separate,
explicitly package-scoped custom-scheme control launched the QA MainActivity.
Commands and unedited outputs are in
[android-dispatch.json](android-dispatch.json). No `pm set-app-links` override or
user-always-open override was used.

**The live issue is not fully resolved by source changes alone.** Current public
Android association fails, and the reported iOS artifact lacks the frontend
entitlements. After merging the source fix, configure the selected hosts with
the approved release identity, verify successful public associations, and ship
a signed native build with the selected frontend host set. Re-test normal HTTPS
dispatch and a cross-domain Safari tap on the reported devices. No production
configuration, certificate trust or deployment was changed here.

## Focused validation

From `mobile/`:

```sh
pnpm exec tsx --test src/lib/loadEnv.test.ts src/lib/linkingPrefixes.test.ts src/app/workspaceCanonicalLinks.test.ts
```

All 51 tests passed. They cover the actual Expo host wiring, shared Activity
paths, default compatibility, profile isolation, explicit frontend requirement,
deduplication, Android opt-in, routing and rejection of untrusted origins.
`node --check mobile/scripts/lib/load-env.js` and `git diff --check` passed from
the repository root. Changed-file TypeScript transpilation passed.
`pnpm exec jest --findRelatedTests app.config.ts scripts/lib/load-env.js --runInBand --passWithNoTests`
found no related Jest tests; the node tests ran explicitly. Full frontend tests, typecheck, native/production builds
and release verification are delegated to CI and the signed-artifact release
process. No full local frontend command ran.
