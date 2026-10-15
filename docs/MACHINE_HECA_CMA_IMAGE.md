# Hosted full Machine image with Heca and CMA Codex

`cli/Dockerfile.machine-hosted` builds the complete hosted Machine directly
from this checkout, including the Heca headless bundle and CMA's Codex provider
binaries and ACP bridge. It owns the hosted build additions, gh, managed profile,
`hosted-entrypoint.sh`, shared desktop startup and managed healthcheck. It reuses
the existing NyxID CLI/Node, Chromium, desktop, cua resources and user separation.
`cli/Dockerfile.machine` and the ordinary container entrypoint are restored to
their original branch-HEAD contents; the hosted build needs no modified base
image. The former `Dockerfile.machine-heca-cma` path is replaced by this target.

Heca and CMA remain on-demand command-user tools. This image does not install CMA's API, database,
`layer4-runtime`, gateway, or other platform services. Building it does not
configure a CMA endpoint, start a provider session, or authorize a Heca host.

## Build

From the NyxID repository root, build the hosted target with a BuildKit named
context for the local CMA checkout:

```sh
docker build \
  --file cli/Dockerfile.machine-hosted \
  --build-context cma="$HOME/Work/Project/aevatarAI/cma" \
  --tag nyxid-machine:hosted-local .
```

The target contains its own CLI builder, full desktop stage and tool stages;
there is no `MACHINE_BASE_IMAGE` build argument or prerequisite base-image build.

For an already downloaded official Heca archive, optionally add
`--build-context public-heca-archive=/absolute/path/to/archive-directory`;
that directory contains `heca.tar.gz` for the target architecture. The directory
is a read-only temporary build mount. Both this input and the default official
release download pass the same architecture-specific pinned SHA-256 check
before extraction. The archive is removed in that build step and is not copied
into the final image. Without this optional context, the empty default stage
selects the original release URL. This is a public build input, not runtime
configuration or account authentication.


Pin the resulting image digest in the managed deployment. Artifact paths and
checksums exist for `linux/amd64` and `linux/arm64`; only architectures with
recorded actual runtime tests may be described as verified.

The reviewed CMA source is commit
`ab789bde158b864d5db467fb2677ef72c3545cfc`. Its Codex adapter directory is mounted
read-only into the build step; the checkout is not copied into the runtime
image. A changed checkout must still produce the exact reviewed ACP output
hash or the build fails. Do not add host home directories, login state, tokens,
Node secrets, or Docker sockets to the build inputs.

| Added component | Pinned version / source |
| --- | --- |
| Heca CLI, daemon and MCP server | `0.1.0-nightly-20260929-1`, official Linux archives from [heca-artifacts](https://github.com/getheca/heca-artifacts) |
| Node.js | `24.19.0`, official Linux archive |
| Codex CLI | `@openai/codex@0.147.0` |
| Codex ACP bridge | `@agentclientprotocol/codex-acp@1.4.0`, followed by CMA's three existing patches |

Heca's public artifact repository identifies `getheca/heca` as the private
source repository. This image consumes the official headless release; it does
not rebuild Heca from private source. The archive installs `heca`,
`heca-daemon`, and `heca-mcp-server` together.

The CMA patches are applied in the same order as CMA's runtime Dockerfile:
`persist_conversations.cjs`, `subscription_auth.cjs`, then
`managed_model_catalog.cjs`. They preserve newly created conversations, add
CMA's subscription-authentication exchange, and support its configured managed
model catalogue. Source and output hashes enforce the reviewed upstream and
patched bridge bytes. Native Codex binaries, the Codex launcher, Heca archives
and Node archives also have explicit checksum checks. Updating a version
requires reviewing the corresponding hashes and patches; overriding a version
argument alone is insufficient. The npm provider install follows CMA's pinned
package versions and selected artifact checks; it does not introduce a lockfile
for the full transitive npm dependency graph.

Node.js is upgraded in the hosted image for the provider runtime. The desktop
stage's npm package manager remains installed; the build-stage npm toolchain
is not copied wholesale into the final image.

## Startup and use

The image selects the complete managed `hosted-entrypoint.sh`. Under managed Kubernetes
bootstrap, NyxID Node still obtains the sealed capabilities and current runtime
identity and waits for the existing execution admission. Heca and Codex are
installed but are **not automatically started or logged in**. They are tools
for the ordinary `agent` command user after the Machine is running and the
caller has the required command permission. The image does not enable Machine
Commands, Browser or Computer capabilities or grant them to a caller.

Use the existing authorized Machine command path to inspect the tools:

```sh
heca --version
heca-daemon --help
heca-mcp-server --help
codex --version
```

The daemon and MCP server do not expose a `--version` option in this Heca
release. Their identity is tied to the verified release archive containing
all three binaries; use their help and startup checks rather than inventing
version output.

For a headless Heca session, explicitly authenticate as the intended user and
start it through the ordinary command-user session:

```sh
heca login
heca daemon start
heca daemon status
# When finished:
heca daemon stop
```

For a local daemon check without a relay connection, use
`heca daemon start --no-relay`. Heca's direct daemon defaults use loopback,
authentication and TLS. No additional port is exposed by this Dockerfile.
Actual Heca account login, remote control and relay use require the service's
own authentication and appropriate permitted outbound network access. They
are not automatically authorized by a NyxID Machine grant. Installation does
not establish an integration between Heca's access policy and NyxID's Machine
grants or selective revocation.

`codex` provides the native CLI. `codex-acp` provides CMA's patched bridge for
a compatible ACP client. This bridge speaks JSON-RPC over stdio; `--help` does
not produce a CLI help response in this pinned build. The smoke test verifies
its `initialize` and session protocols through the real CMA adapter tests. Their real provider use requires explicit runtime
authentication and provider configuration; this image does not supply a CMA
control plane or credential broker. Run them as `agent`, using their supported
authentication paths. Do not supply the protected Node identity, bootstrap
secret or workload token as provider credentials.

No new supervisor or Pod controller is introduced. Heca, Codex and any spawned
processes remain within the Machine container's PID namespace. Node expiry and
container termination retain the inherited lifecycle; normal pause/delete
completion still relies on Agent Sandbox and Kubernetes observations. This
does not add an independent physical-stop guarantee during a node or control
plane partition. Do not detach tools into host services or mount a host Docker
socket to bypass the Machine lifetime.

## Storage and limits

In the existing managed deployment only `/workspace` is persistent. Agent
home, browser homes, desktop/profile state and `/tmp` remain temporary, as
described in the [deployment contract](../deploy/remote-machine/README.md).
Heca and Codex use ordinary user runtime state; with the default temporary home,
login/configuration/session data under that home is not promised to survive
pause or replacement. The image does not redirect account credentials into
the shared workspace. Files intentionally written to `/workspace` retain the
usual workspace lifecycle and access rules. An operator must make any future
credential-persistence decision explicitly; installing these tools does not
create a protected per-tool persistent secret store.

Free and paid hosted Machines use the single `remote-machine` profile, with fixed 2 vCPU, 2 GiB memory and a 20 GiB persistent `/workspace`. Product consent is `remote-machine-mvp`; individual and account deletion remain distinct (`remote-machine-data` and `machine-data`). The full profile supports optional Browser/Computer; default Commands/Files are not its capability ceiling. No undeployed historical profile or policy compatibility remains.

Temporary storage is environment-managed, with no fixed capacity promise, option or extra charge. Both ephemeral-storage resource fields and home/tmp disk emptyDir sizeLimit are absent. Protected identity memory volumes retain their bounds. Pause or Pod replacement may lose temporary state. A retained current Machine preserves its pinned image and workspace even when the deployment default changes. Existing prices, benefits, paused-storage and data-retention policies,
insufficient-balance pause and manual resume remain unchanged. The daemon and
provider share the Machine's resources with Node, browser, desktop and user
jobs. A successful version check or daemon start is not a sustained inference
workload, resource-capacity, or live-cluster acceptance result.

The historical Docker checks below used tmpfs: the extension smoke mounted
`/home/agent` at 512 MiB and `/tmp` at 1 GiB; the complete-image regression
mounted `/home` and `/tmp` at 1 GiB each. These mount types and sizes are local
test conditions, not validation of Kubernetes disk-backed temporary storage
or the current omitted temporary-storage fields. The dated measurements retain their original scope.

> **Historical measured evidence below.** This dated block is preserved byte for byte from the pre-single-current guide. It is not fresh verification of changed Node/profile sources. The single-current stage was recorded separately in `REMOTE_NYX_MACHINE_SINGLE_CURRENT_VERIFICATION_20261015.md`; current hosted/storage results are in `REMOTE_NYX_MACHINE_HOSTED_STORAGE_VERIFICATION_20261015.md`.

## Verification record (2026-10-14)

Actual local build: **Linux arm64**, image `nyxid-machine:heca-cma-local`.

- Complete Machine base: `sha256:361a751b3eca0fe09cdd5acb57c5984f34ef195d626cbd0d75f49eb78d7fc82f`.
- Result: `sha256:7a6430b4cbf0c744993bb2803f6dfffdbad85a38ee8e0985d15bc4c7b90fe1e8`.
- Docker build succeeded after correcting the initial unsupported Heca daemon
  `--version` check to its actual `--help` interface. Pinned download, native
  binary, launcher and patched bridge hash checks passed.
- Image inspection confirms the entrypoint, command, user, healthcheck,
  working directory, volumes, exposed ports and environment match the base.
- Actual retained executables: NyxID CLI/Node `0.69.0`, gh `2.45.0`, Chromium
  `155.0.8059.39`, cua driver `0.31.0`; base npm `9.2.0` still runs with Node
  `24.19.0`. Added Codex is `0.147.0`, ACP `1.4.0` plus the three CMA patches,
  and Heca is `0.1.0-nightly-20260929-1`.

Reproduce the extension's offline runtime verification with the same CMA input:

```sh
sh deploy/remote-machine/tests/heca-cma-image-smoke.sh \
  nyxid-machine:heca-cma-local "$HOME/Work/Project/aevatarAI/cma"
```

This passed using UID/GID 1000, read-only root, no network, no capabilities,
`no-new-privileges`, 2 CPU and 2 GiB memory with no additional swap. Measured
cgroup peak memory was **280,866,816 bytes (267.9 MiB)**. The same smoke fails
against the base image with `codex is missing`, confirming it checks the added
components.

| Check | Observed result / test boundary |
| --- | --- |
| Packaged executables | All existing and added tools found; Node/Codex/Heca versions and final ACP SHA256 matched. No baked Node configuration or Codex account file. |
| Heca Daemon | Real CLI starts the real daemon with `--no-relay`, retaining default local TLS/auth; authenticated status returns the pinned server version and zero agents. Graceful stop exits successfully and subsequent status fails. |
| Heca MCP server | Real stdio `initialize` returns server information while connected to that local daemon. No account login or remote relay. |
| CMA Codex/ACP | **8/8 tests passed** across `persist_conversations`, `subscription_auth`, `managed_model_catalog`, and `live_skill_install`. Real Codex/ACP processes; temporary local model and credential protocol fixtures. |
| CMA supervisor tools | `native_tools.test.cjs` is not run: it requires the intentionally absent `layer4-runtime` and is outside this image's installation contract. |
| Complete Machine regression | **4/4 scenarios passed** on the resulting image: Commands/Files, Browser, all capabilities, and Commands/Files on the next generation with the same workspace. |

Reproduce the inherited complete-image regression with:

```sh
sh deploy/remote-machine/tests/full-machine-image-smoke.sh nyxid-machine:heca-cma-local
```

Its real components are the installed NyxID Node, Chromium, cua, desktop
processes and Docker runtime. Bootstrap, WSS and signed command/control input
come from disposable local fixtures, not a live NyxID backend or Kubernetes
controller. It verifies protected identity boundaries, production healthcheck,
signed commands/files, capability denial, actual Chromium navigation/snapshot,
cua capture/window observation, authority revocation and expiry. The whole
container PID namespace exits after the signed local cutoff, including a
spawned detached command descendant. Heca/Codex are tested separately above;
this inherited regression does not launch an authenticated Heca or model turn.

| Scenario (2 CPU / 2 GiB, no extra swap) | Peak cgroup memory | Container exit after signed cutoff | Exit / OOM |
| --- | ---: | ---: | --- |
| Commands/Files, first generation | 36,409,344 bytes | 2,187 ms | 0 / false |
| Browser only | 621,072,384 bytes | 187 ms | 0 / false |
| Commands/Files/Browser/Computer | 368,058,368 bytes | 2,154 ms | 0 / false |
| Commands/Files, retained workspace, next generation | 35,586,048 bytes | 2,114 ms | 0 / false |

The local Docker volume preserves the workspace marker across two generations;
it does not enforce the real 20 GiB PVC limit. `/home` and `/tmp` are individually
capped at 1 GiB in this regression; this is not whole-Pod ephemeral accounting.
These short smoke measurements do not establish sustained workload capacity.

The first complete-Machine regression attempt failed before starting any
functional tests because of TLS failures to PyPI. The same pinned Linux wheels
(PyNaCl 1.6.2, cffi 2.0.0 and pycparser 2.23) were recovered from the existing
public pip package cache, their metadata and SHA256 recorded, and supplied
through pip's existing `PIP_NO_INDEX` and `PIP_FIND_LINKS` variables. No test or
application behavior was replaced by this dependency transport change.

Real Heca account login/relay/remote use, authenticated Codex inference,
Heca-to-CMA provider/session integration, sustained simultaneous browser plus
agent workloads, and Linux amd64 runtime behavior are **not verified** by these
offline checks. The image does not provision those accounts or services. The
selected Kubernetes runtime, CSI and real 20 GiB PVC enforcement remain
real-environment acceptance items, together with actual namespace admission
defaults, effective Pod resource and volume fields, node disk pressure and
eviction behavior. Check whether external policies inject temporary-storage
values into the v2 template's omitted fields and record the actual effect;
these local checks do not authorize changes to an in-use environment.
Independently enforced physical stopping and storage fencing remain deferred
under the MVP contract; this optional image does not change that boundary.
