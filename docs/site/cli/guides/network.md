---
title: Network, proxies and TLS
description: Configure trusted CA certificates, inspect network failures, and use the CLI behind an HTTP proxy.
---

The CLI verifies server certificates and hostnames for every HTTPS and WSS
connection. It combines bundled Mozilla roots with system roots and any additional CA file.
Bundled and explicitly configured roots load immediately; when no system override
is set, native OS roots load only if certificate verification against the eager
roots fails. The full union is then verified with the same hostname and signature
checks. The native load is cached once per process; ordinary public-CA connections
do not pay for a keychain lookup. Bundled roots remain enabled
in minimal containers that have no CA store.

## Configure trust

| Variable | Meaning |
| --- | --- |
| `NYXID_CA_CERT` | A PEM file containing one or more additional CA certificates. |
| `SSL_CERT_FILE` | A PEM system CA bundle, selected instead of the OS store. |
| `SSL_CERT_DIR` | System CA directories, selected instead of the OS store; separate directories with `:` on Unix or `;` on Windows. |

With neither `SSL_CERT_FILE` nor `SSL_CERT_DIR`, system roots come from the native
platform store, including trusted keychain/MDM roots on macOS. When both variables
are set, certificates from both sources are added. `NYXID_CA_CERT` is always
additive to the bundled and selected system roots. Empty values for any of these
three variables mean unset (for example, `export NYXID_CA_CERT=`).

For an enterprise CA or a TLS-inspecting proxy, obtain the CA from your operator:

```sh
export NYXID_CA_CERT=/absolute/path/company-ca.pem
nyxid doctor
nyxid login --device --no-wait --output json
```

An unreadable, empty, malformed, or invalid-DER file selected by `NYXID_CA_CERT` fails with the
variable and path in the error. Each explicitly configured system source must
provide at least one usable certificate; otherwise the CLI fails and names the
variable. Unparsable individual system certificates are skipped. An unavailable
OS store with no explicit configuration leaves bundled roots enabled and emits
only a debug log. Restart long-running processes after changing the configuration
or certificate files. There is no TLS verification bypass.

## HTTP proxies and WebSockets

HTTPS/HTTP clients retain reqwest's `HTTPS_PROXY`, `HTTP_PROXY`, `ALL_PROXY`, and
`NO_PROXY` behavior, including lowercase forms. These apply to API calls, login,
updates, skill downloads, telemetry, wizard upstream requests, and node HTTP
requests. The existing node OAuth exception is plain HTTP on exact loopback
hosts: it bypasses proxies and pins `localhost` to loopback so local development
credentials cannot travel to a proxy.

**WebSocket connections are direct.** The node agent's server/downstream WSS
connections and `nyxid ssh` share the same CA roots, but do not use HTTP proxy
environment variables. Their hosts must be directly reachable.

`nyxid doctor` and `nyxid doctor --json` force the native-store load and show
bundled and system certificate counts,
selected sources, additional CA counts/errors, one-time native load duration, and
detected proxy variables. Proxy URLs show only scheme, host and port. `NO_PROXY`
and `no_proxy` host patterns are shown with control characters removed and a
300-character limit.

Doctor also checks `GET {base}/health` in its **NyxID API** section, with a
three-second connect timeout and five-second total timeout. Use:

```sh
nyxid doctor --base-url https://nyx-api.chrono-ai.fun --profile dot --json
```

The explicit `--base-url` wins; otherwise doctor uses the selected profile's saved
URL (default profile if omitted), then the CLI's default login URL. The health
probe sends no credentials and strips URL userinfo, query and fragment. Success
reports the sanitized endpoint and HTTP status; failure includes the shared
diagnostic in text and JSON. The Authentication section also inspects that profile.
GitHub failures offer GitHub status/rate-limit guidance separately.
In text output, each failure's diagnostic lines are nested beneath its row:

```text
  NyxID API
    Health                   network check failed                                       ✗
      stage: tls
      endpoint: https://localhost:18443/health
```

## Background node services and automatic updates

At `nyxid node daemon install` or `nyxid update auto enable`, any configured `NYXID_CA_CERT`, `SSL_CERT_FILE`, and
`SSL_CERT_DIR` paths are saved as absolute paths in the LaunchAgent plist or
systemd user unit. Proxy variables are never persisted because they may contain
credentials. Keep CA files readable by the daemon's user.

Reinstall with `nyxid node daemon install --force` to replace the saved values.
Variables omitted from the reinstall environment are removed from the generated
service definition. Restart/start the daemon to apply the definition. On macOS,
`--force` stops the existing process before rewriting the plist; retain your
original `--profile`, customized `--config`, and `--log-level` options.

Re-run `nyxid update auto enable` (with your intended `--interval-hours`) to refresh
the updater's saved CA paths. Unset or empty CA variables remove their previous
entries. Both installers validate explicit trust configuration before installing
the service; node startup also fails immediately on a permanent CA configuration
error instead of reconnecting indefinitely.

`nyxid node docker start` and `nyxid node docker restart` validate the same CA
configuration before changing an existing container. They resolve configured paths
to absolute host paths, bind-mount them read-only, and set container-local values:

| Variable | Container path |
| --- | --- |
| `NYXID_CA_CERT` | `/etc/nyxid/tls/ca.pem` |
| `SSL_CERT_FILE` | `/etc/nyxid/tls/system.pem` |
| `SSL_CERT_DIR` | `/etc/nyxid/tls/certs/0`, `/etc/nyxid/tls/certs/1`, …; joined with `:` inside the Linux container |

```sh
NYXID_CA_CERT="/absolute/path/company ca.pem" nyxid node docker start
```

The commands never forward proxy variables. Re-run `start` or `restart` with the
current CA environment to recreate the container; unset or empty values remove
previous mounts and variables. Spaces and colons in CA file paths are supported
using Docker's `--mount` syntax. In `SSL_CERT_DIR`, the host platform's path-list
separator still separates directories (`:` on Unix, `;` on Windows). CA paths
containing commas, double quotes, or control characters are rejected with the
variable name before the container changes; use paths without those characters.

The bundled image uses Debian bookworm-slim with `ca-certificates` and defaults to
root, which can read owner-only CA files through these mounts. The host paths must
also be accessible to the Docker daemon (including Docker Desktop file sharing).
For manual `docker run --user …`, make CA files readable by that UID and supply
read-only mounts plus the corresponding container-local environment values yourself.
The container uses its own OS CA store; it cannot read the host macOS keychain.
Export any required host-only CA to a PEM file and set `NYXID_CA_CERT`.

## Diagnose login failures

Login error codes, messages and exit statuses are stable. An additive
`error.diagnostic` in JSON identifies the failure stage. Human output prints the
existing message followed by diagnostic lines indented two spaces on stderr. Pending, denied,
expired, already-delivered, invalid-code and rate-limited outcomes retain their
original shape without network diagnostics. An unavailable response may include
the numeric `server_error_code`; server messages are never copied.

| Stage | Action |
| --- | --- |
| `config` | Correct the named CA path/file. |
| `connect` | Check DNS, network and proxy reachability. |
| `proxy` | Check `HTTPS_PROXY`/`NO_PROXY`, proxy authentication, and CONNECT permissions. |
| `tls` | Follow the specific hint: issuer/signature failures need a trusted CA; hostname mismatches need the correct URL/proxy route; expired or not-yet-valid certificates need a certificate/clock check. |
| `request` | Check network/proxy reachability; a timeout identifies `connect` or `request` separately. |
| `response` | Ensure `--base-url` points to a NyxID API and check server health. |
| `validation` | The server returned an invalid verification URL; fix its frontend URL configuration. |
| `storage` | Check local profile-directory permissions and disk space. |

Diagnostics include the HTTP status when available, an endpoint without userinfo,
query or fragment, at most eight sanitized causes (300 characters each, consecutive duplicates removed), and an
actionable hint. Payloads, tokens, login codes, polling secrets and Authorization
headers are excluded. A rejected CONNECT tunnel does not expose its HTTP status
through reqwest's error type; it is still classified as `proxy`.

For example, `login_unavailable` remains exit 19. Resume a pending request after
fixing connectivity; do not mint repeated requests to work around a TLS failure.

## Release verification transport

Release downloads, GitHub attestation requests, and authenticated Sigstore TUF
metadata/target downloads all use the CLI's trust configuration. TUF verification
still enforces signatures, metadata expiration, target hashes and sizes before
Sigstore receives trust-root bytes. Custom TLS roots do not replace the embedded
TUF signing root or the required release workflow identity.
