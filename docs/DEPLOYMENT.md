# NyxID Deployment Guide

This guide covers deploying NyxID in development, staging, and production environments.

---

## Table of Contents

- [Prerequisites](#prerequisites)
- [Local Development](#local-development)
- [Releasing to production](#releasing-to-production)
- [Building for Production](#building-for-production)
- [Docker Deployment](#docker-deployment)
- [Delegated Catalog v2 and Exact Authority v2 Rollout](#delegated-catalog-v2-and-exact-authority-v2-rollout)
- [Environment Configuration](#environment-configuration)
- [Database Setup](#database-setup)
- [RSA Key Management](#rsa-key-management)
- [Reverse Proxy Configuration](#reverse-proxy-configuration)
- [TLS and Certificates](#tls-and-certificates)
- [Frontend Deployment](#frontend-deployment)
- [Health Checks and Monitoring](#health-checks-and-monitoring)
- [Backup and Recovery](#backup-and-recovery)
- [Scaling](#scaling)
- [OIDC Provider Configuration](#oidc-provider-configuration)
- [Initial Admin Setup](#initial-admin-setup)
- [CLI Reference](#cli-reference)
- [MCP Proxy Deployment](#mcp-proxy-deployment)
- [Telegram Bot Setup](#telegram-bot-setup)
- [Security Hardening Checklist](#security-hardening-checklist)
- [Troubleshooting](#troubleshooting)

---

## Prerequisites

| Tool       | Version   | Purpose                              |
|------------|-----------|--------------------------------------|
| Rust       | 1.85+     | Backend compiler (edition 2024)      |
| Node.js    | 20+       | Frontend build tooling               |
| MongoDB    | 7.0+      | Primary database                     |
| Docker     | 24+       | Container runtime (optional)         |
| openssl    | 3.x       | Key generation                       |

---

## Local Development

### 1. Start infrastructure

```bash
docker compose up -d
```

This starts:
- **MongoDB 8.0** on `127.0.0.1:27017` (credentials: `nyxid` / `nyxid_dev_password`)
- **Mailpit** SMTP on port `1025`, web UI on port `8025`

### 2. Configure environment

```bash
cp .env.example .env
```

The default `.env.example` is pre-configured for local development. Replace the placeholder `ENCRYPTION_KEY`:

```bash
# Generate a real encryption key
openssl rand -hex 32
```

Paste the output as `ENCRYPTION_KEY` in `.env`.

### 3. Start the backend

```bash
cargo run --manifest-path backend/Cargo.toml
```

The backend starts on `http://localhost:3001`. In development mode:
- RSA signing keys are auto-generated in `keys/` if missing
- MongoDB collections and indexes are created automatically
- Email verification tokens are logged to the console

### 4. Start the frontend

```bash
cd frontend
npm install
npm run dev
```

The frontend starts on `http://localhost:3000`.

### 5. Verify

```bash
curl http://localhost:3001/health
# {"status":"ok","version":"0.5.0"}
```

---

## Releasing to production

Feature PRs target `dev`. Pushes there trigger the staging build and deploy
workflow for the backend and frontend. To promote the commit running on both, preview the rollup with
`scripts/cut-release.sh --dry-run`, then run `scripts/cut-release.sh` to open
the snapshot PR. The snapshot stays fixed while work continues on `dev`.

**Main Merge Guard** accepts three sources from this repository for a PR into
`main`. Each PR needs one approval, full **CI Pipeline**, a passing guard and
**Create a merge commit**.

| Source | What the guard checks |
| --- | --- |
| `dev` | The source is the integration branch. |
| `release/YYYY-MM-DD` | The date is valid and the head is already in `dev` history. An optional numeric suffix starts at `-2`. |
| `hotfix/<name>` | The head contains the current `main` tip and every commit it adds is absent from `dev`. The name has 1–64 ASCII characters, starts with a lowercase letter or digit, and may also contain `.`, `_` and `-`. |

For an urgent production fix, branch from current `main` and open the hotfix
PR back into `main`. If `main` advances during review, update the hotfix and
rerun its checks. After it lands, open a PR **from `main` into `dev`** and
choose **Create a merge commit** so the hotfix keeps its commit ancestry.
The next rollup must use a snapshot containing that sync. Its notes follow
the first parent of `dev` and list the sync PR once.

### Automatic production deploys

In GitHub, create the `production` environment with a deployment branch rule
for `main` only. Store `NYXID_PRODUCTION_DEPLOY_CLIENT_ID` and
`NYXID_PRODUCTION_DEPLOY_CLIENT_SECRET` as environment secrets for the
NyxID service account authorized to deploy the two production apps through
the deployment proxy (OAuth scope `proxy`). The staging job reads
`NYXID_STAGING_DEPLOY_CLIENT_ID` and `NYXID_STAGING_DEPLOY_CLIENT_SECRET`
from the `staging` environment.

Set `NYXID_DEPLOY_TARGET` separately in each GitHub environment's secrets.
Supply a single-line JSON object with the following required fields.

| Field | Meaning |
| --- | --- |
| `apps_url` | Full apps collection URL for the deployment project. The job appends the app name for updates and the app name followed by `/status` for readiness checks. |
| `org_id` | Namespace scope sent with each deployment request. |
| `backend.app`, `frontend.app` | App name in the corresponding component's JSON object. |
| `backend.service`, `frontend.service` | Kubernetes service whose image the job updates and whose pod status it reads. |
| `backend.version_url` | Backend health URL that reports status and the running commit. |
| `frontend.version_url` | Frontend build-version URL that reports the running commit. |

Use HTTPS URLs without credentials, query strings or fragments. App and service
names start with an ASCII letter or digit, may also contain dots, underscores
and hyphens, and have a 63-character limit. The namespace scope also permits
colons after the first character and has a 128-character limit.

Open **Actions → Publish Images → the main run → deploy-production** for
progress. A push to `main` or **Run workflow** on `main` deploys once the
CI gate and multi-architecture manifests succeed. Version tags only publish
images. Production deploy jobs serialize. Each compares the live versions
to the candidate before updating either app and checks the app again before
its PATCH. A newer observed commit stops the run. A divergent or unknown
commit fails it.

The job deploys the backend first. It requires three consecutive ready
samples, ten seconds apart, with the candidate commit and backend health
status `ok`. It then checks both apps together as the frontend rolls out.
For a failed or partial rollout, use **Re-run failed jobs** on that run.
An app already serving the candidate is checked for readiness and skipped
for update.

### NyxBot deployment check

The staging and production deploy jobs hold their existing deployment slot
through a private NyxBot check. After both apps serve the candidate, the job
signs in on the backend health URL's HTTPS origin, submits one owner invocation
to a fixed private thread, requires a completed root turn with reply text,
then sends one fixed message to a private test channel. The channel must
return a platform message ID. The check has a 90-second deadline. An uncertain
admission or unfinished turn gets one bounded cancellation request by its
original idempotency key. The check never retries a paid turn or a send.

Configure each GitHub environment separately. Use a dedicated, unprivileged
owner with a hard-capped model budget, a private model-only specialist thread,
and one opted-in channel route whose only recipient is a test account. Enable
owner invocations for this owner after the fleet supports them. See
[owner invocation rollout](chat/09-owner-cli-api.md#rollout-and-rollback).

| Environment setting | Value |
| --- | --- |
| Secret `NYXID_CHECK_ACCOUNT` | JSON object with string fields `email`, `password`, `owner_id` (UUID returned by first-party CLI login), `conversation_id` (private specialist thread) and `channel_id` (channel conversation UUID). |
| Secret `NYXID_CHECK_ALERT_URL` | HTTPS incoming webhook accepting a Slack-style JSON `text` field. Use an independent release on-call destination. |
| Variable `NYXID_ROLLBACK_PAIR` | Optional compatibility approval in the form `<previous full SHA>..<candidate full SHA>`. An empty or stale value permits checks and blocks automatic restoration. |

When check credentials are configured and both apps need updating, the job
resolves both published manifests and pins each PATCH to its digest. The
passing artifact name and JSON record the two digests actually applied. If
registry lookup is unavailable, the usual SHA-tag rollout and check can
proceed. Only a passing pair deployed by digest establishes a rollback target.

Missing check secrets produce a visible skip. A passing check produces a JSON
artifact retained for 90 days. Each record contains UTC time, environment,
candidate commit, outcome, stage and rollback disposition. Failures keep the
deploy job red, including after successful recovery. The alert includes the
run link. Use the failed run's artifact and summary for incident history.

Before arming a pair, review **every** intervening release and the environment's
actual migrations, feature activations, workers and external writers against
the rollback rules in `CLAUDE.md`. Examples include exact billing cutover, new
billing allocation writers and the first owner invocation. Verify the previous
binary supports all state the candidate or check can write. Coordinate manual
deploys and hold incompatible activations through the deploy/check/restore
window. Leave the variable empty for a forward-fix release or uncertain fleet
state. Configure the pair before the deploy job starts.

On failure the job requires the same predecessor SHA observed at **both** live
version endpoints before rollout and immediately before their respective updates.
It verifies commit ancestry, the exact compatibility approval and an unexpired passing
check artifact from the latest run and attempt of that SHA's branch
publish workflow. That run must have succeeded. Both live apps must still serve the failed candidate.
Restoration uses the same deployer and the two recorded digests, updating the backend first
and verifying both apps together when the frontend is ready. A changed live
pair, missing proof, expired receipt, partial start or already-current start
blocks automatic restoration. An ambiguous PATCH stops the sequence. Inspect
the reported disposition before rerunning a failed deployment since a rerun
can redeploy the failed candidate.

### Rolling back a production release

Coordinate the rollback with any active or queued Publish Images deploys.
In the deployment platform, select the project and namespace identified by
`apps_url` and `org_id` in the production environment's `NYXID_DEPLOY_TARGET`.
Choose the seven-character SHA of a known-good, rollback-compatible `main`
build. Update the backend and frontend apps and services named in the
environment's `NYXID_DEPLOY_TARGET` in this order.

1. Set the backend image to `ghcr.io/chronoaiproject/nyxid/backend:main-<sha7>`.
   Wait for the app to report ready and for `backend.version_url` to return
   `status: ok` and the expected 12-character commit.
2. Set the frontend image to `ghcr.io/chronoaiproject/nyxid/frontend:main-<sha7>`.
   Verify the same commit at `frontend.version_url`.

Land the revert or fix on `main` so the next automatic deployment carries it.

---

## Building for Production

### Backend

```bash
# Release build (optimized, no debug symbols)
cargo build --release --manifest-path backend/Cargo.toml

# Binary output
ls -la backend/target/release/nyxid-server
```

The release binary is a single statically-linked executable (with dynamically linked system libraries). No runtime dependencies beyond the OS.

### Frontend

```bash
cd frontend
npm ci
npm run build

# Output in frontend/dist/
ls -la frontend/dist/
```

The build produces static files (HTML, JS, CSS) ready to serve from any CDN or static file server.

---

## Docker Deployment

Production Dockerfiles are provided for both backend and frontend with multi-stage builds, dependency caching, and non-root users.

### Building Images

```bash
# Build backend (context is the repo root for workspace Cargo.toml)
docker build -f backend/Dockerfile -t nyxid-backend .

# Build frontend (context is the repo root so the build can read docs/site,
# the source for the in-app docs pages)
docker build -f frontend/Dockerfile -t nyxid-frontend .
```

The **backend Dockerfile** (`backend/Dockerfile`) builds the Rust binary and a
separate Node 22.23.2 credential-accept bundle. Its Debian runtime runs as
non-root `nyxid` and includes the standalone HTML, SRI assets and release
manifest at `/app/frontend/dist/`; it also needs its private durable TUF volume.

Backend builds accept `--build-arg NYXID_GIT_HASH=<commit>`. The final image sets
that value as a runtime environment variable for `/health`; it is not an input
to either Cargo layer. Without a nonempty runtime value, standalone server
binaries retain their build-time stamp. CLI/node binaries retain their existing
build-time identity. The credential-accept bundle still receives its own commit
stamp and release-integrity manifest.

A separate Python recipe stage copies only workspace manifests and `Cargo.lock`
and normalizes the nine workspace packages to version `0.0.0` in both. It leaves
third-party versions, checksums and requirements exact. The fresh `dependencies`
stage copies only that normalized output and generates stub crate entry points;
a version-only release changes the recipe stage but its output still has the
same content hash. Normalizing in the compiler stage itself would not work: its
parent COPY layer would already have changed. The real `builder` restores the
original manifests/lock before compiling application code, preserving `/health`
and every `CARGO_PKG_VERSION`. New versioned path constraints or inherited member
versions fail the recipe explicitly until supported; they are never relaxed.
Both Cargo invocations use `--locked` and the same release binary/features.
Real source and embedded assets enter only after
dependency compilation. With those inputs unchanged, a commit-only change now
reuses the compiled server layer; source/embedded-input changes still rebuild
the affected crates, and dependency/toolchain changes invalidate the dependency
layer. Workspace package version bumps alone do not. This is layer reuse, not
incremental Rust compilation.

`CARGO_BUILD_JOBS` is a build argument with a default of **2**, used by both the
amd64 smoke job and native amd64/arm64 publishing. Each backend image runner
first provisions at least 24 GiB total swap. Two jobs permit bounded dependency
and code-generation parallelism without using every runner CPU; the large
server crate compiles after its dependencies, and the CLI/test binaries are not
built. The swap reserve absorbs peaks rather than making sustained swapping
fast. Smaller local builders can pass `--build-arg CARGO_BUILD_JOBS=1`. No change
to optimization, release features or runtime resources is made.

Publishing imports and exports registry caches at
`ghcr.io/<owner>/nyxid/backend:buildcache-linux-{amd64,arm64}` with `mode=max`,
including intermediate compiler layers. Both jobs retain the matching
`backend-linux-<arch>` GHA import as a migration fallback. Smoke also imports the
old `backend-pr-linux-amd64` scope, but exports no cache: in PR #2052 the release
compile finished in 49m 23s, then even a `mode=min` GHA export stalled for nearly
40 minutes until the 90-minute job timeout. That export did not retain
intermediate compiler layers. PRs never overwrite the trusted registry cache
or consume the shared Actions cache budget, where large image exports previously
evicted Rust release caches. A missing/cold registry cache still requires a full
compile. The first publish after this Dockerfile change repopulates it.

For a bounded dependency-cache check, build with `--target dependencies`, then
repeat after changing only workspace package versions in the manifests/lock.
The dependency Cargo step must say `CACHED`. On small local builders add
`--build-arg CARGO_BUILD_JOBS=1`; this target never compiles the real server.
Local native arm64 validation on 2026-10-27 compiled dependencies in 8m 10s.
Changing all nine local package versions to `9.8.7` then completed in 3s with
the Cargo layer `CACHED`; both invocations retained `--locked`.

For timings, use `docker buildx build --progress=plain` with the same registry
`--cache-from` as publishing, then repeat with only `NYXID_GIT_HASH` changed.
Both Cargo `RUN` steps should report `CACHED` on the second build; the credential
bundle stamp and final image metadata still change. Backend source changes
cannot get that complete compiler-layer hit. No fixed speedup is assumed for
cold builds or the single large crate: compare Cargo's reported build duration
and BuildKit's per-layer durations on each native runner.

The **frontend Dockerfile** (`frontend/Dockerfile`) uses a two-stage build:
1. Node 22.23.2 builder with `npm ci` and `npm run build`
2. Unprivileged nginx 1.27 runtime with a validating Python origin/policy renderer

The frontend nginx config (`frontend/nginx.conf.template`) handles:
- SPA fallback (`try_files $uri /index.html`)
- Reverse proxy for `/api`, `/oauth`, `/.well-known`, `/health` to the backend
- Gzip compression and cache headers for hashed assets
- Response-specific enforcing CSP and complete security headers on every location

See [Production browser policy](BROWSER_SECURITY_POLICY.md) for the resource
inventory, validated `NYXID_PUBLIC_ORIGIN`/`NYXID_BROWSER_CONNECT_ORIGINS`, HSTS
boundary, credential routing, access-log redaction and production browser tests.
Use its generated frontend configuration for direct static hosting too;
handwritten static-server snippets below require the same response policies.

The production SPA checks `/health` and `/build-version.json` on startup, then
every ten minutes. Returning to the tab or reconnecting cannot trigger another
background check within that ten-minute window.
Frontend metadata is authoritative because frontend and backend images can roll
independently. Two consecutive observations of a different frontend build stage
its initial JavaScript and CSS in the browser cache, without executing scripts,
attaching styles, or remounting the running application. The served `index.html`
must also identify that build before it is considered ready.

Confirmed updates are saved in a Zustand store persisted to local storage, with
an earliest detection timestamp that later polls and superseding deployments
preserve. A matching loaded build clears the record; observing that the deployed
build matches the running build also cancels it. A saved pending build must still
match live metadata and available assets before activation.

When assistant code changes, deliberate chat selection, New chat, agent/group
selection, and ordinary links to another view attempt an update after navigation
completes. Eligible destinations are reading pages and the main assistant,
plugins, approvals, automations and machines listings; live desktops, setup and
authentication flows are excluded. The selected destination remains in the URL
through the reload.
Automatic conversation adoption and programmatic redirects do not trigger this.
Activation rechecks the deployment and refuses to reload if a local direct-chat
turn is running/waiting, a mutation or dialog is active, editable controls contain
text, the tab is hidden, or the destination changes during validation. Automatic
activation uses the same session reload-loop guard as hidden-tab updates.

Every confirmed frontend build still pending after 15 minutes shows a dismissible
refresh banner, including on `/assistant` and its workspace routes. The banner
does not filter by assistant fingerprint or change size. Dismissal applies only
to the displayed build; a later frontend build can show its own reminder. The
15-minute delay starts when the first pending update has been confirmed and its
assets checked, after the two consecutive observations described above. Later
deployments retain that deadline.

The assistant fingerprint controls automatic activation on safe view switches.
This SHA-256 fingerprint covers the assistant's resolved source dependencies, the
bootstrap and root-shell dependencies, source styles, audio worklet, build
configuration and scripts, and the dependency lockfile. Other routes are pruned
at the lazy page barrel. Raw source bytes avoid changes caused only by a new
commit stamp or regrouped output chunks; project version bumps are ignored in
the lockfile. Public build environment changes also count. This identifies
frontend assistant changes; `/health` does not expose a separate backend
assistant version. Fingerprints are activation hints and do not retain old
lazy assets across deployments.

An untouched tab on the landing, dashboard, docs, blog or legal reading pages can
refresh automatically after at least one minute hidden. Any pointer, keyboard,
form, drop or SPA navigation activity disables automatic refresh for that
document. Dialogs, editable controls, nonempty search boxes, pending mutations,
query strings and URL fragments also block it. Stateful pages such as assistant
chat, terminals, authentication and one-time credential flows never reload merely
because the tab becomes hidden. Assistant updates instead use deliberate view
switches when safe, with the delayed banner as a fallback. The banner's Refresh
button checks the deployment again before
reloading. Failed polls back off to thirty minutes; session storage bounds automatic
reloads, including mixed replicas and rollback loops. Development builds do not poll.

The image publish workflow supplies `NYXID_GIT_HASH` to the frontend Docker build,
which stamps both the bundle and its version metadata. For manual image builds,
pass `--build-arg NYXID_GIT_HASH=<commit>` to get the same stamp; without it each
build uses a timestamp identity. Deploy this feature once before existing tabs
can detect subsequent deployments. A visible document reload still rechecks
authentication and loads route data, so immediate new code with zero visual
change is not guaranteed. Keeping previous hashed assets available during
rollouts also avoids the existing lazy-chunk recovery refresh when an old tab
navigates to a route whose chunk has been removed.

### Production Docker Compose

The production stack is defined as an override layer on top of the base
`docker-compose.yml`. Both files must be passed together with `-f`; the
dev `docker-compose.override.yml` is intentionally NOT auto-loaded when
`-f` is used explicitly.

```bash
# If you previously ran the quickstart, remove the dev symlink first
[ -L .env.production ] && rm .env.production

# Create a production env file from the template
cp .env.production.example .env.production
$EDITOR .env.production
# Set the 3 required values: MONGO_ROOT_PASSWORD, ENCRYPTION_KEY, BASE_URL

# Generate RSA keys for JWT signing
mkdir -p keys
openssl genrsa -out keys/private.pem 4096
openssl rsa -in keys/private.pem -pubout -out keys/public.pem
chmod 755 keys
chmod 644 keys/private.pem keys/public.pem
# 644 lets the non-root `nyxid` user inside the backend container read the
# bind-mounted keys when host UID != container UID (common on Windows WSL2).
# For tighter perms in production, align host ownership with the container
# `nyxid` user — see "RSA Key Management" below.

# Pull published images and start the stack
docker compose -f docker-compose.yml -f docker-compose.prod.yml \
               --env-file .env.production pull
docker compose -f docker-compose.yml -f docker-compose.prod.yml \
               --env-file .env.production up -d --scale backend=2
```

The combined config starts four containers by default:

- **backend**: starts two replicas, mounts `keys/` read-only, and waits for MongoDB health. The service has no fixed container name or host port. Docker DNS distributes frontend requests across the replicas.
- **frontend**: binds `127.0.0.1:3000` and proxies API, WebSocket, and MCP traffic to the backend service through `BACKEND_URL`.
- **mongodb**: persistent volume, health check via `mongosh`, password sourced from `MONGO_ROOT_PASSWORD` in `.env.production`

Change the backend count with `--scale backend=N`. Keep the frontend as the
only published application port. Each backend derives its instance name and
private advertise URL from that container's unique `HOSTNAME`.

Multi-arch (linux/amd64, linux/arm64) images are published to GHCR by the
`Publish Images` workflow on every merge to main and every `v*` tag. Pin to
a specific version by exporting `NYXID_VERSION=v1.2.3` before running compose.

### Pushing to a Registry

```bash
# Tag and push to your container registry
docker tag nyxid-backend registry.example.com/nyxid/backend:v0.5.0
docker tag nyxid-frontend registry.example.com/nyxid/frontend:v0.5.0

docker push registry.example.com/nyxid/backend:v0.5.0
docker push registry.example.com/nyxid/frontend:v0.5.0
```

---

## Kubernetes Deployment

This repository does not ship a `k8s/` directory or maintained Kubernetes manifests. Earlier instructions referenced files that are absent and have been removed. Kubernetes deployment requires operator-authored manifests and validation; it is not an installable repository example.

Use the production configuration and scaling guidance in this document to define Deployments, Services, secret mounts, database replica-set connectivity, health checks, long-lived WebSocket/MCP routing and TLS termination. Keep the backend internal port private and restricted to replicas; never expose it through public ingress. Share the supported key/configuration set across replicas. Validate your actual ingress headers and network policies. Maintained manifests are a separate deployment deliverable.

---

## Delegated Catalog v2 and Exact Authority v2 Rollout

This change is safe under an ordinary rolling backend deployment. No traffic
pause, approval drain, or coordinated Aevatar release is required:

- The delegated discovery contract remains
  `nyxid-delegated-operation-catalog.v2`. The new `risk`,
  `supports_idempotency_key`, and nullable `operation_generation` fields are
  additive. During the bounded compatibility window, discovery publishes the
  pre-additive v2 digest and new replicas accept both that digest and the full
  additive projection. Existing clients and replicas therefore continue to
  create and revalidate the same fence while new clients can consume the added
  metadata. New approval rows keep that pre-additive digest in the legacy
  `exact_view_digest` slot and also persist the full projection in the
  server-owned `exact_view_digest_binding` slot. Old replicas validate the
  former; new replicas enforce both, so an old writer changing additive policy
  fields without a generation bump is still detected.
- New approvals store both execution-authority projections: the real v1 digest
  in `execution_authority_digest` and the versioned v2 binding in
  `execution_authority_binding`. An old replica validates the v1 slot exactly
  as before; a new replica validates v2. Existing main-era rows with only a v1
  digest are compared against the live v1 projection, and older rows with no
  digest retain their expiry-bounded behavior.
- Producer generation is resolved by the server. Durable endpoint rows bind
  their positive generation; instance-mounted specification operations have no
  durable generation and keep the endpoint-contract digest as their shape
  fence.

Deploy backend replicas with the normal rolling strategy and verify `/health`
reports the intended release commit as each replica becomes ready. A rollback
is also safe: old code can validate the real v1 slot written by new replicas.

Startup checks the exact-approval semantic-effect unique index. Duplicate
legacy data or a duplicate write racing the index build no longer blocks
process startup: NyxID logs the audited remediation, leaves the index absent,
and activates a diagnostic on the admin Integrity page. The service-level
semantic replay checks remain active until an operator reconciles the duplicate
groups and restarts to install the unique index.

---

## Rolling deploys and graceful drain

Server mode installs SIGTERM and ctrl-c receivers before configuration or database
IO, and retains the same receivers through shutdown. Startup never cancels an
awaited step on a signal. It checks at three safe points: after configuration,
after the read-only database connection and transaction-capability check, and
immediately before either listener binds. At the first two points it exits with
status 0 without application writes or workers. After the database checkpoint,
startup finishes through the bind checkpoint, including migrations and worker
initialization. A pending signal there prevents both binds and the process-start
audit, runs the existing drain for workers already started, then exits with
status 0. A second signal is retained during startup and skips the drain wait
once that checkpoint is reached; it never interrupts a startup step.

For a startup signal, the configured drain budget starts at signal receipt.
Remaining startup consumes that budget: with budget B and remaining startup S,
at most max(B - S, 0) remains for drain and cleanup. This does not place a deadline
on startup itself. If a startup operation stalls beyond B, the process reaches
cleanup late, with no budget remaining; a platform kill can still interrupt it.
Under the normal startup duration (less than ten seconds), the default 25-second
budget includes the remaining startup and drain. One-shot commands install no
server signal receivers; database command modes also start no billing or chain
verification workers. After binding, shutdown follows the serving behavior below.

SIGTERM or ctrl-c marks the process as draining. The unauthenticated `GET /ready`
returns 503 immediately; `GET /health` keeps its existing 200 liveness response.
Both public and internal replica listeners continue serving during the work wait,
including ordinary requests and new turns that arrive while routing converges.
There is no drain rejection on these request paths and `/ready` never accesses
the database. The staging platform's readiness checks and load balancer retain
this drain-only, empty-body contract. chrono-probe can use the same `/ready` signal;
MongoDB availability is monitored separately (see
[Health and readiness contract](HEALTH_READINESS.md)).

The process stops taking new background queue claims, scheduled work and polls.
Already claimed work finishes, with turn heartbeats, capacity renewal, node
ownership renewal and security capability observations kept alive. The wait
counts detached turns and delivery work as well as HTTP bodies and upgraded
streams. Idle node management sockets are closed at handover; their pending
operations keep the wait open. MCP notification subscriptions, assistant `/live`, and dedicated CMA agent/response
event observers do not count as work. They end cleanly five seconds after
drain starts, allowing readiness propagation before reconnecting. MCP clients
resume the durable outbox with `Last-Event-ID`; assistant clients poll and refresh
on `ready`/`resync` after reconnect. Dedicated CMA observers display Disconnected
and retain the cursor for their existing Reconnect action; closing observation
does not cancel the remote task. Generic proxy streams remain work because their
upstream resume guarantees vary. An otherwise idle drain waits for those bodies
to close. Turn/provider streams, downloads, managed setup results, native response
replay delivery barriers and interactive sessions still wait for completion and
can consume the full work-wait budget. Once work is idle, Axum finishes accepted connections while
retaining the bound sockets. If an accepted request starts more work at that
boundary, both listeners resume serving until that work finishes.

`SHUTDOWN_DRAIN_TIMEOUT_SECS` defaults to **25 seconds**. The default fits
Kubernetes' default 30-second termination grace period and needs no setting.
The budget includes cleanup and the finish audit: the smaller of five seconds
or 10% is reserved for cleanup. At the default, subscriptions end at five seconds,
the work wait ends at 22.5 seconds, and remaining turns are interrupted with
`turn_lost` / `shutdown`. Remaining local execution and response streams stop
after at most another 0.5 seconds of ordinary settlement. Fenced settlement,
release and the finish audit share the 2.5-second cleanup reserve; the process
exits by the 25-second deadline, leaving five seconds for signal delivery,
scheduling and process teardown before Kubernetes kills it. The asynchronous
start audit has its own 1.25-second bound within this budget, not extra exit time.
A blocking preStop hook must not consume the five-second margin.

Work that needs longer than the budget when a deploy starts is ended and recorded
as interrupted by shutdown. To let that work finish across a deploy, raise both
values together: for an extended budget, the termination grace period must exceed
the drain budget by at least 30 seconds. For example, set
`SHUTDOWN_DRAIN_TIMEOUT_SECS=600` and use this Deployment fragment:

```yaml
spec:
  template:
    spec:
      terminationGracePeriodSeconds: 630
      containers:
        - name: nyxid
          env:
            - name: SHUTDOWN_DRAIN_TIMEOUT_SECS
              value: "600"
```

The process exits earlier when work finishes. A second SIGTERM or ctrl-c skips
the remaining wait and begins the same bounded cleanup. Explicit timeout values
retain their existing behavior; `0` restores the old immediate-exit path,
including its existing ten-second Discord cancellation allowance.

At the end, runners settle remaining turns through the existing fenced path
with stable code `turn_lost` and stage `shutdown`. Cleanup cancels remaining local
execution, attempts settlement for this worker's remaining claims, closes node
WebSockets with going-away code 1001, releases owned cluster leases and capacity
slots without deleting polling checkpoints, and removes this generation's
security observation after stopping its writer. Releases compare generations
or exact claim tokens. One metadata-only `backend_shutdown_drain` chained audit
and structured log describe the outcome, including the count of subscriptions
ended by shutdown. Logs at drain start show the signal, budget and in-flight
counts; another log records a second signal. Database failures and stuck cleanup
have a shared deadline; they cannot extend the exit budget.

The chained, metadata-only `backend_shutdown_drain_started` and
`backend_shutdown_drain` events share the process's unique `generation`. The start
records the budget, cleanup reserve and in-flight counts, including node operations.
A start without a finish indicates the process was killed before the drain
completed (a failed finish audit can also leave only a start); compare `budget_secs`
with the platform's termination grace period. `0` emits no drain events.

Start and finish records also include `open_http_at_start` and, at finish,
`open_http_at_finish`: the largest 20 route-template/method/kind groups, their
counts and oldest ages in whole seconds, plus `other_count` for omitted units.
Groups also distinguish the resolved, unowned catalog `service` slug for active
public `connection`/`internal` discovery entries regardless of creator (excluding
internal `platform-*` and model-only rows, with empty/UUID-shaped slugs and every
other resolved service reported as `custom`, omitted before resolution) on
proxy/public/LLM routes and the fixed stream `content_type` family (`event-stream`,
`json`, `other`), never private service slugs, user connection aliases or raw
content-type headers.
Kinds distinguish pending requests from response bodies/upgraded streams; age
starts when that kind is registered. Raw paths, IDs, queries, headers and extension
methods are never captured (`<unmatched>` / `OTHER` are fixed fallbacks).
Detached/background guards have no static labels, so retain aggregate counts.
While draining, `backend_shutdown_drain_progress` records elapsed time, counts,
node operations and the same `open_http` summary at 10, 25, 60 and 300 seconds,
skipping points at or beyond the budget (only 10 seconds applies by default).
These at-most-four chained appends use
the start audit's time bound and are cancelled before finish; they cannot keep
work non-idle or extend cleanup. The last successful checkpoint bounds how far
a killed drain got, though an audit failure can also explain a missing checkpoint.

After both listeners bind, `backend_process_started` records the generation,
build, bind time, and the latest other generation and heartbeat observed under
the same instance name (if any), through the chained audit path; its background
lookup is bounded to one second and the entire attempt to three seconds, without
holding up serving. `previous_lookup` distinguishes `found`, `none`, and
`unavailable`; a retained heartbeat is evidence of a prior process, not proof it
was still alive, and gracefully retired observations are removed. Two process
starts without an intervening drain for the earlier generation indicate an
unobserved exit (including a crash or kill); a missing audit or a crash before
listener binding can leave no start record, so one start alone does not prove a
routing fault. Drain start, progress and finish records include `recent_traffic`:
at most twelve five-second buckets with UTC boundaries and aggregate `received`
and `responded` counts across both listeners, including probes and errors.
`responded` counts middleware response-header completion, not body completion or
confirmed client delivery; the newest bucket is partial and concurrent snapshots
are approximate. Counting uses fixed process-local atomics and no request data or
database calls; only those drain snapshots persist the rolling history.

Cleanup does not clear uncertain provider-send barriers, wallet locks or
purchased-credit expiry operations. Interrupted rotating OAuth refreshes, channel-webhook setup and Telegram setup
retain their mutation locks; Discord identify rate windows retain their expiry. These records
keep their existing recovery rules because releasing them could repeat an
external effect. `billing:reconcile` uses its own cooperative stop and exact-token
release after the current financial unit completes. If the deadline aborts that
pass, its lease retains the default 30-second TTL; bulk shutdown release excludes
it. Credit-schedule progress leases retain their existing 300-second expiry
because they have no exact ownership token. Completion tasks, including detached audits, compensation and
ledger appends, remain tracked while execution is stopped.

Operators must match these settings in their platform's rollout configuration:

- The default 25-second drain fits Kubernetes' default 30-second grace period.
  For an extended drain budget, set the grace period at least 30 seconds higher;
  allow the legacy Discord allowance when using `0`.
- Readiness probe on `/ready`; liveness probe on `/health`.
- Replace one replica at a time with surge capacity, allowing the successor to
  become ready before terminating another replica.
- Keep the draining replica's internal address reachable until it exits, because
  peers forward node operations to the process that owns the socket.

These are configuration requirements, not a description of production rollout
tooling. That tooling is outside this repository. If the platform kills the
process earlier, or MongoDB cannot confirm settlement within cleanup, peers use
the unchanged stale-heartbeat recovery: 75 seconds plus the 15-second sweep
cadence. A worker loss never automatically replays execution. A fully drained
turn has its complete reply and needs no recovery.

### Background turn rollback

Enrollment in `nyxagent-reattach-v1` defaults off. An admin can enable
`assistant:turn-handoff` for a person, an organization
or globally in Feature Flags after every NyxID replica advertises
`assistant:turn-handoff-v2` and every NyxAgent replica advertises
`reattach.credential_watch`. Enrollment requires live fleet readiness, the
watch capability and a selected profile with `inference_credential: thread`.

Standalone NyxBot specialist assignments and member turns in personal web groups
also require `assistant:turn-handoff-v3` across the serving NyxID fleet. They
use the same feature flag. The survivor takes over the original team slot,
publishes an assignment to its saved report thread, or settles the member reply
and mention budget together in the group transcript. A pending group follower
receives its ordinary `group_settled` event when the group becomes quiet.

Owner-private chat turns and NyxBot event wakes require `assistant:turn-handoff-v4`
on every serving replica. A survivor uses the admitted channel or team slot and
the turn's saved routing. The existing event delivery worker owns the primary chat
reply. A wake's supplemental replies carry their original destination through
settlement. Their delivery workers skip confirmed parts and preserve uncertain
sends for review. Personal manager chats use their admitted manager transport
for discovery, replay and cancellation. Guest, voice, shared-agent, organization
and collaboration lanes stay excluded.

The admitting replica discovers missing metadata with the owner's existing
private assistant credential. It allows two seconds in total before admission
for `GET /v1/capabilities` and `GET /v1/models?include_capabilities=true`.
The replica reuses deployment capabilities and the owner's profiles for up to
60 seconds. Browser, mobile and automation clients can start a turn directly.
If discovery lacks an authorized credential or times out, the turn proceeds
in the foreground.

Group `Assistant turn settled` logs by `restart_eligibility`. Count
`watched_thread_credential` for enrolled turns. `capability_unavailable`
indicates missing or incompatible deployment evidence, `profile_unknown`
indicates insufficient evidence for the selected profile, and
`platform_credential` identifies a profile using a platform key.
`lane_ineligible` identifies a turn excluded by the lane check. `discovery_skipped`
means the admission caller supplied no discovery snapshot. `upstream_refused_background`
identifies a turn that fell back after an upstream refusal. The field is absent
when the feature flag or fleet readiness gate skips negotiation.

Disable every enabling override in Feature Flags **before rolling NyxAgent back**
to a version without the credential watch. Allow 30 seconds for admission hints
to expire and let admissions already in flight finish. Keep enrollment disabled until every NyxAgent
replica advertises `reattach.credential_watch` again. Model discovery can
precede a profile reload, and a different replica can admit the request.

Before rolling back a fleet that has enrolled turns, disable every enabling
override, allow 30 seconds for admission hints to expire and let admissions already in
flight finish. Keep compatible subscribers and cancellation handlers serving.
Run `mongosh "$NYXID_MONGODB_URI" --quiet --file scripts/mongodb/check-turn-handoff-rollback.js`
with a URI selecting the NyxID database. Exit 0 means the majority read on the
primary found zero outstanding conversation records and zero saved chat replies
awaiting delivery. Exit 2 prints counts for subscribed, offered, claimed or
stopped windows, grouped by the active lane when present, and the number of
pending saved deliveries. Keep v4 subscribers and delivery workers running until
both counts reach zero before rolling back to v3. Run this check before removing v3 subscribers even when rolling
back to a v2-capable build. V2 readers decode the compatible source and skip
version 3 in both offer discovery and the claim update. A read error requires another
check. A stopped window clears after confirmed cancellation or after the next
authorized admission replaces the internal credential. Deleting its marker by
hand could let an earlier execution use the next turn's authority.

Admins can read `assistant_turn_handoff` entries in the audit log. Each committed
offer, claim and settlement records the generation of the pod that offered the
turn. A second handoff has its own offering generation. Count transitions by
generation in the NyxID database with the following read-only aggregation.

```javascript
db.audit_log.aggregate([
  { $match: { event_type: "assistant_turn_handoff" } },
  { $group: { _id: { generation: "$event_data.generation",
                     transition: "$event_data.transition", outcome: "$event_data.outcome" },
              count: { $sum: 1 } } },
  { $sort: { "_id.generation": 1, "_id.transition": 1, "_id.outcome": 1 } }
], { readConcern: { level: "majority" } })
```

## Environment Configuration

### Required Variables

| Variable         | Description                                    | Example                                         |
|------------------|------------------------------------------------|-------------------------------------------------|
| `DATABASE_URL`   | MongoDB connection string                      | `mongodb://user:pass@host:27017/nyxid?authSource=admin` |
| `ENCRYPTION_KEY` | 32-byte hex-encoded AES-256 key (64 hex chars) | Output of `openssl rand -hex 32`                |
| `ENCRYPTION_KEY_PREVIOUS` | Previous encryption key for key rotation (64 hex chars, optional) | Old `ENCRYPTION_KEY` value during rotation. Phase 1 supports one previous key at a time. |

### Server Variables

| Variable       | Default                 | Production Example               |
|----------------|-------------------------|----------------------------------|
| `PORT`         | `3001`                  | `3001`                           |
| `BASE_URL`     | `http://localhost:3001` | `https://auth.example.com`       |
| `FRONTEND_URL` | `http://localhost:3000` | `https://app.example.com`        |
| `ENVIRONMENT`  | `development`           | `production`                     |

### JWT Variables

| Variable               | Default            | Recommendation                    |
|------------------------|--------------------|-----------------------------------|
| `JWT_PRIVATE_KEY_PATH` | `keys/private.pem` | `/etc/nyxid/keys/private.pem`     |
| `JWT_PUBLIC_KEY_PATH`  | `keys/public.pem`  | `/etc/nyxid/keys/public.pem`      |
| `JWT_ISSUER`           | Same as `BASE_URL` | Leave unset (uses `BASE_URL`)     |
| `JWT_ACCESS_TTL_SECS`  | `900` (15 min)     | `900` or lower                    |
| `JWT_REFRESH_TTL_SECS` | `604800` (7 days)  | `604800` or lower                 |

### Rate Limiting

| Variable                | Default | Recommendation                            |
|-------------------------|---------|-------------------------------------------|
| `RATE_LIMIT_PER_SECOND` | `10`    | Adjust based on expected traffic          |
| `RATE_LIMIT_BURST`      | `30`    | Set higher for bursty workloads           |

### SMTP (Required for Email Features)

| Variable            | Description               | Example                     |
|---------------------|---------------------------|-----------------------------|
| `SMTP_HOST`         | SMTP server hostname      | `smtp.sendgrid.net`         |
| `SMTP_PORT`         | SMTP server port          | `587`                       |
| `SMTP_USERNAME`     | SMTP username             | `apikey`                    |
| `SMTP_PASSWORD`     | SMTP password             | `SG.xxxxx`                  |
| `SMTP_FROM_ADDRESS` | Sender email address      | `noreply@example.com`       |

### Social Login (Optional)

| Variable               | Description             |
|------------------------|-------------------------|
| `GOOGLE_CLIENT_ID`     | Google OAuth client ID  |
| `GOOGLE_CLIENT_SECRET` | Google OAuth secret     |
| `GITHUB_CLIENT_ID`     | GitHub OAuth client ID  |
| `GITHUB_CLIENT_SECRET` | GitHub OAuth secret     |

### Logging

| Variable   | Default                          | Production                       |
|------------|----------------------------------|----------------------------------|
| `RUST_LOG` | `nyxid=info,tower_http=info`     | `nyxid=info,tower_http=warn`     |

---

## Database Setup

### MongoDB Requirements

- Version 7.0 or higher (8.0 recommended)
- Authentication enabled
- TLS connections in production

### Connection String Format

```
mongodb://username:password@host:27017/nyxid?authSource=admin&tls=true
```

For replica sets:

```
mongodb://user:pass@host1:27017,host2:27017,host3:27017/nyxid?authSource=admin&replicaSet=rs0&tls=true
```

### Automatic Setup

NyxID creates all required collections and indexes automatically on first startup via `db::ensure_indexes()`. No manual migration steps are needed for fresh installations.

### Upgrading large databases

Before rolling out this version to a large database, pre-build the 23 erasure
and export indexes using the exact owner and export-selector commands in the
[personal-erasure inventory](security/PERSONAL_ERASURE_INVENTORY.md).
This is required for large collections such as `refresh_tokens`, `nyxbot_events`,
`assistant_messages`, `trigger_deliveries`, and channel receipts. Startup awaits
these builds before either listener binds, so a long first build can exhaust
readiness/startup probe allowances. Pre-building lets the current replicas keep
serving during the build; startup then reuses the compatible existing indexes.
No existing index needs to be dropped, renamed, or rebuilt.

### Collections Created

NyxID uses 29 MongoDB collections, all created automatically at startup:

| Collection               | Purpose                                      |
|--------------------------|----------------------------------------------|
| `users`                  | User accounts (email, password hash, MFA status) |
| `sessions`               | Server-side sessions with hashed tokens      |
| `oauth_clients`          | Registered OAuth/OIDC clients (includes delegation_scopes) |
| `authorization_codes`    | Short-lived OIDC authorization codes         |
| `refresh_tokens`         | Refresh tokens with rotation chain tracking  |
| `api_keys`               | User-scoped API keys (hashed, with prefix)   |
| `downstream_services`    | Registered downstream services for proxying  |
| `user_service_connections` | Per-user connections and encrypted credentials |
| `mfa_factors`            | TOTP factors and encrypted recovery codes    |
| `service_endpoints`      | Registered API endpoints per service (MCP tools) |
| `provider_configs`       | External provider registry (encrypted OAuth creds) |
| `user_provider_tokens`   | Per-user encrypted provider tokens (API keys/OAuth) |
| `user_provider_credentials` | Per-user encrypted provider credentials    |
| `service_provider_requirements` | Provider token requirements per service |
| `oauth_states`           | Temporary OAuth state for provider flows     |
| `roles`                  | Role definitions with permissions and scoping |
| `groups`                 | Group definitions with role inheritance      |
| `consents`               | User OAuth consent records per client        |
| `service_accounts`       | Non-human (machine) identity definitions     |
| `service_account_tokens` | Issued service account JWT records for revocation |
| `approval_requests`      | Pending/resolved approval requests for proxy access |
| `approval_grants`        | Cached approval grants (time-limited, revocable) |
| `service_approval_configs` | Per-service approval overrides (per user)  |
| `notification_channels`  | Per-user notification preferences, Telegram links, push device tokens |
| `nodes`                  | Registered credential nodes (per user, with auth token hash and status) |
| `node_service_bindings`  | Service-to-node routing bindings             |
| `node_registration_tokens` | One-time tokens for node registration (TTL-indexed) |
| `mcp_sessions`           | MCP protocol session state                   |
| `audit_log`              | Immutable audit trail of security events     |

### MongoDB Atlas

For managed MongoDB (Atlas):

1. Create a cluster (M10+ for production)
2. Create a database user with `readWrite` role on the `nyxid` database
3. Whitelist your server IP addresses
4. Use the provided connection string with `tls=true`

---

## RSA Key Management

NyxID signs JWTs with RS256. Generated key pairs use 4096-bit RSA; imported signing pairs and retained verification keys must contain at least 3072 bits. Startup validates that the signing private and public keys match. A partially missing pair is an error; development generation does not replace a surviving key.

### Generating Keys

```bash
# Generate private key
openssl genrsa -out keys/private.pem 4096

# Extract public key
openssl rsa -in keys/private.pem -pubout -out keys/public.pem

# Restrict permissions (private key should be read-only by the app)
chmod 600 keys/private.pem
chmod 644 keys/public.pem
```

### Development Mode

In development (`ENVIRONMENT=development`), NyxID auto-generates keys if the configured paths do not exist. This is disabled in production.

### Production Key Management

- Store keys outside the application directory (e.g., `/etc/nyxid/keys/`)
- Use filesystem permissions to restrict access (`chmod 600`)
- Mount keys as read-only volumes in Docker
- Rotate keys with an overlap of prepared verification keys, using the sequence below
- Back up the private key securely -- losing it invalidates all issued JWTs

### Key Rotation

JWT verification supports the active signer plus up to sixteen retained public keys through `JWT_RETAINED_PUBLIC_KEY_PATHS`. NyxID publishes their public keys in JWKS. Verification selects the exact known `kid` and requires RS256; missing or unknown key IDs fail without probing other keys.

1. Generate the replacement pair in separate files. Keep the current signer active.
2. Configure the replacement public key as a retained verifier on every replica and deploy all readers before changing any signer. Confirm that external JWT consumers can also consume the overlapping JWKS.
3. Activate the replacement pair. Retain the old public key in `JWT_RETAINED_PUBLIC_KEY_PATHS` on every replica throughout this rollout.
4. Keep the old verifier for the lifetime of every supported JWT class issued by the old signer, including verification leeway and any required external-consumer overlap. Retained public keys do not extend token expiry or bypass live user/session/consent checks.
5. Remove a verifier only after its required overlap has elapsed. Removing it immediately invalidates still-valid JWTs naming that key. Independent Machine-provider trust roots are not rotated by this setting.

Private signing-key backups remain sensitive even after the public verifier is retired. This procedure describes the implemented configuration; no deployment or key retirement is performed by the remediation PR.

---

## Reverse Proxy Configuration

NyxID should run behind a reverse proxy in production for TLS termination, load balancing, and `X-Forwarded-For` header injection.

### Account export transport

Do not compress `POST /api/v1/users/me/export` at any ingress, reverse proxy or
CDN. Preserve its exact `Content-Length`, disable response buffering, and propagate
upstream body errors so interrupted exports cannot be saved as complete. The
frontend nginx configuration has an exact export location with gzip and buffering
disabled; apply the same rule to any additional ingress. If an intermediary does
encode a response, clients cannot compare its wire length to decoded bytes and
must fall back to JSON or structural validation.

### Caddy (Recommended)

```
auth.example.com {
    reverse_proxy localhost:3001

    header {
        X-Forwarded-For {remote_host}
    }
}

app.example.com {
    root * /var/www/nyxid/frontend/dist
    file_server

    # SPA fallback
    try_files {path} /index.html
}
```

Caddy handles TLS certificates automatically via Let's Encrypt.

### Nginx

```nginx
upstream nyxid_backend {
    server 127.0.0.1:3001;
}

server {
    listen 443 ssl http2;
    server_name auth.example.com;

    ssl_certificate     /etc/letsencrypt/live/auth.example.com/fullchain.pem;
    ssl_certificate_key /etc/letsencrypt/live/auth.example.com/privkey.pem;

    # Security headers (NyxID also sets these, but defense in depth)
    add_header X-Frame-Options DENY always;
    add_header X-Content-Type-Options nosniff always;

    # Proxy to backend
    location / {
        proxy_pass http://nyxid_backend;
        proxy_set_header Host $host;
        proxy_set_header X-Real-IP $remote_addr;
        proxy_set_header X-Forwarded-For $proxy_add_x_forwarded_for;
        proxy_set_header X-Forwarded-Proto $scheme;

        # WebSocket support (if needed in future)
        proxy_http_version 1.1;
        proxy_set_header Upgrade $http_upgrade;
        proxy_set_header Connection "upgrade";
    }
}

# Frontend static files
server {
    listen 443 ssl http2;
    server_name app.example.com;

    ssl_certificate     /etc/letsencrypt/live/app.example.com/fullchain.pem;
    ssl_certificate_key /etc/letsencrypt/live/app.example.com/privkey.pem;

    root /var/www/nyxid/frontend/dist;
    index index.html;

    # SPA fallback
    location / {
        try_files $uri $uri/ /index.html;
    }

    # Cache static assets
    location ~* \.(js|css|png|jpg|jpeg|gif|ico|svg|woff2?)$ {
        expires 1y;
        add_header Cache-Control "public, immutable";
    }
}

# Redirect HTTP to HTTPS
server {
    listen 80;
    server_name auth.example.com app.example.com;
    return 301 https://$host$request_uri;
}
```

---

## TLS and Certificates

### Requirements

- TLS 1.2+ (TLS 1.3 preferred)
- Valid certificate from a trusted CA (Let's Encrypt, etc.)
- HSTS is emitted only for configured HTTPS public origins (`BASE_URL` on the
  backend, `NYXID_PUBLIC_ORIGIN` on frontend nginx), independent of request
  Host/forwarded-proto headers. Neither subdomain inclusion nor preload is automatic.

### Let's Encrypt with Certbot

```bash
# Install certbot
sudo apt install certbot python3-certbot-nginx

# Obtain certificates
sudo certbot --nginx -d auth.example.com -d app.example.com

# Auto-renewal (cron)
sudo certbot renew --dry-run
```

### Cookie Security

NyxID automatically sets the `Secure` flag on authentication cookies when `BASE_URL` does not start with `http://localhost` or `http://127.0.0.1`. Ensure `BASE_URL` uses `https://` in production so that cookies are transmitted only over TLS.

---

## Frontend Deployment

### Static Hosting

The frontend build output (`frontend/dist/`) is a static SPA. Deploy it to any static hosting provider:

- **CDN**: Cloudflare Pages, Vercel, Netlify
- **Object Storage**: S3 + CloudFront, GCS + Cloud CDN
- **Self-hosted**: Nginx, Caddy, Apache

### SPA Routing

The frontend uses client-side routing. Configure your server to serve `index.html` for all routes that don't match a static file (see the Nginx and Caddy examples above).

### Environment at Build Time

The frontend API URL is configured in `frontend/src/lib/api-client.ts`. For production, update this to point to your backend's public URL before building:

```bash
# Build with production API URL
cd frontend
npm run build
```

### Cache Strategy

- HTML files: no-cache (always fetch latest)
- JS/CSS with content hashes: cache forever (`Cache-Control: public, max-age=31536000, immutable`)
- Vite automatically adds content hashes to built assets

---

## Health Checks and Monitoring

For the frontend/backend endpoint matrix, finite JSON failure reasons, actual
production/staging base URLs, chrono-probe assertions and staged rollout, see
[Health and readiness contract](HEALTH_READINESS.md). Frontend `/health` retains
its backend/version proxy; frontend-only probes are `/frontend/health` and
`/frontend/ready`. The staging deployment platform and chrono-probe use backend
`/ready` only for drain status. Both backend probes operate independently of MongoDB.
Production Compose keeps backend `/health` and checks frontend `/frontend/ready`
over loopback, exempt from its public rate/concurrency limits. Frontend
orchestrators and load balancers must use `/frontend/health` for liveness and
routing; `/frontend/ready` is for chrono-probe and local container healthchecks
only, and must not drive replica eviction.

Publish Images may reuse successful CI jobs from an identical pull-request
merge tree on `main`; see [Main publish gate reuse](ci/MAIN_GATE_REUSE.md).
Any uncertainty runs the complete release gate.

### Health Endpoint

```bash
curl https://auth.example.com/health
# {"status":"ok","version":"0.5.0"}
```

Use this for:
- Liveness monitoring (load balancer readiness uses `/ready`)
- Container orchestration liveness probes
- Uptime monitoring (Uptime Robot, Pingdom, etc.)

### Kubernetes Probes

```yaml
livenessProbe:
  httpGet:
    path: /health
    port: 3001
  initialDelaySeconds: 5
  periodSeconds: 10
readinessProbe:
  httpGet:
    path: /ready
    port: 3001
  timeoutSeconds: 3
  initialDelaySeconds: 5
  periodSeconds: 5
```

### Logging

NyxID uses structured logging via `tracing`. Configure log levels with `RUST_LOG`:

```bash
# Development (verbose)
RUST_LOG=nyxid=debug,tower_http=debug

# Production (essential only)
RUST_LOG=nyxid=info,tower_http=warn

# Debug specific modules
RUST_LOG=nyxid::services::auth_service=debug,nyxid=info
```

Logs are written to stdout in a human-readable format. For JSON-structured logs suitable for log aggregation (ELK, Datadog, etc.), modify the tracing subscriber in `main.rs` to use `.json()` format.

### Audit Log

NyxID maintains an internal audit log in the `audit_log` MongoDB collection. Query it via the admin API:

```bash
curl "https://auth.example.com/api/v1/admin/audit-log?page=1&per_page=50" \
  -H "Authorization: Bearer <admin_token>"
```

The Audit Log page uses `include_total=false` to display filtered rows before
their exact count completes. This mode returns `has_more` and fetches at most
one extra row to determine whether another page exists. `total` is omitted
when it would require a separate filtered count. Unfiltered totals still use
collection metadata, and a complete first page can supply its total directly.
The UI then requests `GET /api/v1/admin/audit-log/count` with the same filters;
this endpoint returns `{ "total": 123 }` and requires the same admin/operator
access. It performs no display-enrichment lookups. Omitting `include_total`
preserves the existing listing response with its total.

Frontend and backend can be upgraded independently. Existing clients keep their
numeric `total` unless they explicitly opt into `include_total=false`; `has_more`
is additive. The updated UI accepts older responses without `has_more` and uses
their total without requesting the count endpoint. During a mixed-replica
rollout, an unavailable count endpoint leaves the page usable with a retry for
the total. Counts may be cached for 60 seconds; the current page's `has_more`
controls forward pagination.

For an indexed identity lookup, use the `user_id` or `api_key_id` query parameter,
or select **User ID (exact)** / **API key ID (exact)** in the page's search field
selector. Text searches retain substring matching and may scan many audit rows.

New audit rows are HMAC hash-chained with a monotonically increasing `seq`. Verify the chain on demand:

```bash
curl "https://auth.example.com/api/v1/admin/audit-log/verify?limit=10000" \
  -H "Authorization: Bearer <admin_token>"
```

Rows created before hash chaining are reported as `pre_chain_count`. This v1 chain detects edits, non-tail deletion gaps, forged inserts, and reordering by a database-only attacker, but it does not detect deleting the newest N rows until the chain head is anchored outside MongoDB.

---

## Backup and Recovery

### MongoDB Backup

```bash
# Full backup using mongodump
mongodump --uri="mongodb://user:pass@host:27017/nyxid?authSource=admin" \
  --out=/backups/nyxid-$(date +%Y%m%d)

# Restore from backup
mongorestore --uri="mongodb://user:pass@host:27017/nyxid?authSource=admin" \
  /backups/nyxid-20260101/
```

### Automated Backup Schedule

```bash
# Cron job: daily backup at 2am, retain 30 days
0 2 * * * mongodump --uri="$DATABASE_URL" --out=/backups/nyxid-$(date +\%Y\%m\%d) && find /backups -maxdepth 1 -name "nyxid-*" -mtime +30 -exec rm -rf {} \;
```

### What to Back Up

| Item             | Location                 | Frequency |
|------------------|--------------------------|-----------|
| MongoDB database | Via `mongodump`          | Daily     |
| RSA key pair     | `keys/private.pem`       | Once (store securely) |
| Environment file | `.env.production`        | On change |

### Recovery Steps

1. Restore MongoDB from backup (`mongorestore`)
2. Place RSA keys at the configured paths
3. Restore the environment file
4. Start the NyxID backend -- indexes are recreated automatically
5. Verify with `curl /health`

---

## Scaling

### Multiple backend replicas

NyxID keeps durable application and coordination state in MongoDB. Credential
node WebSockets and MCP SSE response channels remain attached to one process.
MongoDB records which process owns each live connection, so a request can land
on any replica.

```text
                              public :3001
Ingress -> nyxid-backend --------------------------+
                 |                                 |
                 +--> replica A <----+----> replica B
                       :3002         |       :3002
                                      |
                                  MongoDB
```

The public `nyxid-backend` ClusterIP is the only ingress target. Replica owners
advertise their pod IP through `Node.connection_owner`. The private
`nyxid-backend-headless` Service exposes port 3002 inside the cluster for
cross-replica dispatch. Each internal request uses a timestamped HMAC and a
MongoDB-backed nonce check. A NetworkPolicy allows port 3002 only between
backend pods. Do not expose port 3002 through an ingress.

The owner record includes an instance name, a process generation, and a socket
connection ID. These values fence stale processes and replaced sockets. The
owner renews a 90-second lease every 30 seconds. After a crash, the lease
expires and a reconnecting node can claim ownership on another replica.

MongoDB also provides the shared OAuth refresh leases, Telegram polling lease,
DPoP replay records, event dedup claims, rate windows, and capacity slots.
Configured authentication, per-key, per-agent, platform-service, WebSocket,
and SSH limits apply to the whole cluster. `NODE_MAX_WS_CONNECTIONS` is the one
intentional per-replica limit because it protects that process's file
descriptors and memory.

MCP sessions use MongoDB as the authority. SSE notifications use a durable
MongoDB outbox, so a POST can land on a different replica from the SSE stream.
The MCP ingress cookie keeps clients on one replica when possible and reduces
outbox polling latency. Correctness does not depend on that cookie.

All replicas must:

- Share the same RSA key pair and encryption keys.
- Connect to the same MongoDB deployment.
- Use the same coordination TTLs and the same internal HMAC key.
- Advertise a private URL that other replicas can reach.
- Keep clocks synchronized within `INTERNAL_AUTH_MAX_SKEW_SECS`.

The manifests start two backend replicas. The node WebSocket and MCP
Ingress resources use 3,600-second read and send timeouts. `/mcp` uses prefix
routing, so its session subpaths reach the backend.

See [Horizontal scaling architecture](HORIZONTAL_SCALING_ARCHITECTURE.md) for
the ownership and failure semantics. See [Environment variables](ENV.md#cluster-coordination)
for every coordination setting.

### MongoDB scaling

- **Replica Set**: For high availability (3+ nodes recommended)
- **Sharding**: Not required at typical auth workloads (millions of users are fine on a single replica set)
- **Read Preferences**: Use `secondaryPreferred` for read-heavy admin/audit queries

---

## Initial Admin Setup

NyxID ships without a default admin account. You must create the first admin using one of the two methods below.

### Option 1: Bootstrap Endpoint (Recommended for First Deploy)

When the database is empty (zero users), NyxID exposes a one-time setup endpoint:

```bash
curl -X POST http://localhost:3001/api/v1/auth/setup \
  -H "Content-Type: application/json" \
  -d '{
    "email": "admin@example.com",
    "password": "secureadminpassword123",
    "display_name": "Admin"
  }'
```

This creates a user with `is_admin = true` and `email_verified = true`. The endpoint automatically locks itself after the first user is created -- subsequent calls return `403 Forbidden`.

### Option 2: CLI Promote (For Existing Users)

If a user has already registered through the normal flow, promote them to admin via the command line:

```bash
# Using cargo
cargo run --manifest-path backend/Cargo.toml -- --promote-admin admin@example.com

# Using the built binary
./backend/target/release/nyxid-server --promote-admin admin@example.com
```

This sets `is_admin = true` and `email_verified = true` on the user. The command exits after completion (does not start the server).

### Verification

After creating the admin, verify by logging in and accessing an admin endpoint:

```bash
# Login as admin
curl -X POST http://localhost:3001/api/v1/auth/login \
  -H "Content-Type: application/json" \
  -c cookies.txt \
  -d '{"email": "admin@example.com", "password": "secureadminpassword123"}'

# Access admin endpoint
curl http://localhost:3001/api/v1/admin/users \
  -b cookies.txt
```

---

## CLI Reference

NyxID supports command-line flags for administrative operations.

```
nyxid [OPTIONS]

Options:
    --promote-admin <EMAIL>    Promote an existing user to admin by email, then exit
    -h, --help                 Print help
    -V, --version              Print version
```

### --promote-admin

Promotes an existing user to admin by their email address. Requires a running MongoDB instance (reads `DATABASE_URL` from environment or `.env`).

```bash
nyxid --promote-admin user@example.com
```

Behavior:
- Finds the user by email (case-insensitive)
- Sets `is_admin = true` and `email_verified = true`
- Logs an `admin_promoted` audit event
- Prints the user ID on success
- Exits with code 0 on success, 1 on failure
- Does not start the HTTP server

Errors:
- `No user found with email: ...` -- The email is not registered
- `User ... is already an admin` -- The user already has admin privileges

---

## OIDC Provider Configuration

NyxID acts as a full OpenID Connect (OIDC) identity provider. Downstream services can delegate authentication to NyxID using the standard Authorization Code flow with PKCE.

### How It Works

1. An admin creates a downstream service in NyxID with `auth_type: "oidc"`.
2. NyxID auto-provisions an OAuth client and generates a `client_id` and `client_secret`.
3. The downstream service uses the OIDC discovery endpoint to auto-configure itself.
4. Users authenticate via NyxID and are redirected back to the downstream service with an authorization code.
5. The downstream service exchanges the code for access, refresh, and ID tokens.

### Step 1: Create an OIDC Service

```bash
curl -X POST https://auth.example.com/api/v1/services \
  -H "Authorization: Bearer <admin_access_token>" \
  -H "Content-Type: application/json" \
  -d '{
    "name": "Customer Portal",
    "base_url": "https://portal.example.com",
    "auth_type": "oidc"
  }'
```

NyxID returns the service with an `oauth_client_id`. The default redirect URI is set to `https://portal.example.com/callback`.

### Step 2: Retrieve OIDC Credentials

```bash
curl https://auth.example.com/api/v1/services/<service_id>/oidc-credentials \
  -H "Authorization: Bearer <admin_access_token>"
```

Response includes:
- `client_id` -- The OAuth client identifier
- `client_secret` -- The client secret (store securely)
- `redirect_uris` -- Registered callback URLs
- `issuer`, `authorization_endpoint`, `token_endpoint`, `userinfo_endpoint`, `jwks_uri` -- OIDC endpoints

### Step 3: Configure Redirect URIs (Optional)

If the default `{base_url}/callback` is not correct, update the redirect URIs:

```bash
curl -X PUT https://auth.example.com/api/v1/services/<service_id>/redirect-uris \
  -H "Authorization: Bearer <admin_access_token>" \
  -H "Content-Type: application/json" \
  -d '{
    "redirect_uris": [
      "https://portal.example.com/auth/callback",
      "https://portal.example.com/api/auth/callback"
    ]
  }'
```

### Step 4: Configure the Downstream Service

The downstream service uses the OIDC discovery URL to auto-configure the authorization flow. The discovery document is available at:

```
https://auth.example.com/.well-known/openid-configuration
```

### OIDC Protocol Details

| Parameter            | Value                                                |
|----------------------|------------------------------------------------------|
| Flow                 | Authorization Code with PKCE (S256)                  |
| Response Type        | `code`                                               |
| Grant Types          | `authorization_code`, `refresh_token`                |
| Token Signing        | RS256 (generated RSA 4096-bit; imported minimum 3072-bit)                    |
| Scopes               | `openid`, `profile`, `email`                         |
| Client Auth Methods  | `client_secret_post`, `none` (for public clients)    |
| ID Token Claims      | `sub`, `iss`, `aud`, `exp`, `iat`, `email`, `email_verified`, `name`, `picture`, `nonce` |

### Example: NextAuth.js Configuration

```javascript
// pages/api/auth/[...nextauth].js
import NextAuth from "next-auth";

export default NextAuth({
  providers: [
    {
      id: "nyxid",
      name: "NyxID",
      type: "oidc",
      issuer: "https://auth.example.com",
      clientId: "<client_id>",
      clientSecret: "<client_secret>",
      checks: ["pkce", "state"],
    },
  ],
});
```

NextAuth.js will automatically fetch the discovery document from `https://auth.example.com/.well-known/openid-configuration` and configure all endpoints.

### Example: Passport.js Configuration

```javascript
// auth.js
const { Strategy } = require("openid-client");
const { Issuer } = require("openid-client");

async function setupAuth(app) {
  const nyxidIssuer = await Issuer.discover("https://auth.example.com");

  const client = new nyxidIssuer.Client({
    client_id: "<client_id>",
    client_secret: "<client_secret>",
    redirect_uris: ["https://portal.example.com/callback"],
    response_types: ["code"],
  });

  app.use(
    "/auth",
    new Strategy({ client, usePKCE: true }, (tokenSet, userinfo, done) => {
      done(null, userinfo);
    })
  );
}
```

### Secret Rotation

To rotate an OIDC client secret without downtime:

1. Regenerate the secret via the API:
   ```bash
   curl -X POST https://auth.example.com/api/v1/services/<service_id>/regenerate-secret \
     -H "Authorization: Bearer <admin_access_token>"
   ```
2. Update the downstream service configuration with the new `client_secret`.
3. Deploy the downstream service. The old secret is immediately invalidated.

---

## MCP Proxy Deployment

The MCP proxy exposes NyxID-managed downstream services as Model Context Protocol (MCP) tools, allowing AI assistants to interact with registered APIs through a single authenticated endpoint.

### Architecture

```
AI Client --[MCP/HTTP]--> MCP Proxy --[OAuth 2.1]--> NyxID Backend --[proxy]--> Downstream APIs
```

The proxy authenticates users via NyxID's OIDC endpoints and dynamically generates MCP tools from the service endpoints registered in NyxID.

### Configuration

| Variable | Description | Example |
|---|---|---|
| `NYXID_URL` | NyxID backend URL | `http://backend:3001` (Docker) or `https://auth.example.com` |
| `NYXID_CLIENT_ID` | OAuth client ID registered in NyxID | `mcp-proxy` |
| `NYXID_CLIENT_SECRET` | OAuth client secret | (from NyxID admin panel) |
| `MCP_PORT` | Port for the MCP HTTP server | `3002` |

### Setup

1. Register an OAuth client in NyxID for the MCP proxy (type: confidential, scopes: `openid profile email`).
2. Configure the environment variables (see above).
3. Start the proxy:

```bash
# Development
cd mcp-proxy
cp .env.example .env
# Edit .env with your NyxID credentials
npm install
npm run dev
```

> **Note:** The standalone `mcp-proxy` service is not currently bundled in
> `docker-compose.prod.yml`. The backend exposes MCP directly at `/mcp`, so
> most deployments do not need a separate proxy. If you need a standalone
> proxy, build and run it manually until it is added to the compose stack.

### MCP Client Configuration

MCP clients connect to the proxy's Streamable HTTP endpoint:

```
POST http://localhost:3002/mcp
```

The proxy serves OAuth 2.1 Protected Resource Metadata at:

```
GET http://localhost:3002/.well-known/oauth-protected-resource
```

MCP clients that support OAuth (e.g., Claude Desktop) will automatically discover NyxID as the authorization server and prompt the user to authenticate via the browser.

### Lazy Tool Loading

To optimize performance, NyxID implements dynamic tool loading:

**Session Initialization:**
- MCP sessions start with built-in meta-tools and no activated per-service tools. Core meta-tools include:
  - `nyx__search_tools` -- Search connected service tools by keyword
  - `nyx__discover_services` -- Browse unconnected services (does not activate)
  - `nyx__list_connected_services` -- Enumerate connected services and availability (does not activate)
  - `nyx__connect_service` -- Connect to a specific service and activate its tools
  - `nyx__call_tool` -- Invoke a connected tool and activate its service

**On-Demand Activation:**
- When the LLM calls `nyx__connect_service`, that service's tools are activated
- When the LLM calls `nyx__call_tool`, the invoked tool's service is activated
- `nyx__search_tools`, `nyx__discover_services`, and `nyx__list_connected_services` do not activate service tools
- Clients (Cursor, Claude Code) automatically refresh their tool list when they receive the notification

**Constraints:**
- Maximum 20 activated services per session (bounded memory usage)
- `nyx__discover_services` is browse-only and does NOT activate tools
- `nyx__list_connected_services` is browse-only and does NOT activate tools

### How Tools Are Generated

For each active downstream service with registered endpoints, the proxy dynamically creates MCP tools following this pattern:

- **Tool name**: `{service_slug}__{endpoint_name}` (e.g., `stripe__list_customers`)
- **Description**: `[{service_name}] {endpoint_description}`
- **Input schema**: Derived from endpoint parameters and request body schema

Only services with valid connections and satisfied credentials are included:
- `connection` services require the user to have stored an encrypted per-user credential
- `internal` services only require an active connection record
- `provider` services are excluded (not proxyable)

### Backward Compatibility

The REST endpoint `/api/v1/mcp/config` returns the same caller-scoped executable operation set used by stateless MCP `tools/list`, plus stable opaque identities, a canonical content digest, response media/binary metadata, and non-sensitive diagnostics. Scoped callers never receive excluded service/node identities or exclusion counts.

When a tool is called, the proxy:
1. Resolves the service and endpoint from the tool name.
2. Substitutes path parameters from tool arguments (URL-encoded for safety).
3. Separates remaining arguments into query parameters and request body.
4. Forwards the request through NyxID's authenticated proxy (`/api/v1/proxy/{service_id}/{path}`).

### Client Configuration: Claude Code

Add the NyxID MCP server to your Claude Code settings (`~/.claude/settings.json` or project `.mcp.json`):

```json
{
  "mcpServers": {
    "nyxid": {
      "url": "https://mcp.example.com/mcp"
    }
  }
}
```

Claude Code supports OAuth-based MCP servers natively. On first use, it will open a browser window for authentication via NyxID.

### Client Configuration: Cursor

Add the NyxID MCP server to your Cursor settings (`.cursor/mcp.json`):

```json
{
  "mcpServers": {
    "nyxid": {
      "url": "https://mcp.example.com/mcp"
    }
  }
}
```

Cursor will detect the OAuth Protected Resource Metadata at `/.well-known/oauth-protected-resource` and prompt for authentication automatically.

### Health Check

```bash
curl http://localhost:3002/health
# {"status":"ok","sessions":0}
```

### Reverse Proxy

If exposing the MCP proxy externally, add it to your reverse proxy configuration:

```
# Caddy
mcp.example.com {
    reverse_proxy localhost:3002
}
```

```nginx
# Nginx
server {
    listen 443 ssl http2;
    server_name mcp.example.com;

    location / {
        proxy_pass http://127.0.0.1:3002;
        proxy_set_header Host $host;
        proxy_set_header X-Real-IP $remote_addr;
        proxy_set_header X-Forwarded-For $proxy_add_x_forwarded_for;
        proxy_set_header X-Forwarded-Proto $scheme;
        proxy_http_version 1.1;
        proxy_set_header Connection "";
        proxy_read_timeout 300s;
    }
}
```

Note the extended `proxy_read_timeout` -- MCP tool calls that proxy to slow downstream APIs may take longer than the default timeout.

---

## Credential Broker Setup

NyxID's credential broker feature lets users store encrypted API keys and OAuth tokens for external providers (e.g., OpenAI, Anthropic, Google AI). These tokens are injected into proxy requests on behalf of users.

### Prerequisites

The credential broker reuses the existing `ENCRYPTION_KEY` and `BASE_URL` environment variables. No additional configuration is needed.

### Step 1: Configure Providers

After the initial admin setup, register provider configurations via the API:

**API key provider (e.g., OpenAI):**

```bash
curl -X POST https://auth.example.com/api/v1/providers \
  -H "Authorization: Bearer <admin_access_token>" \
  -H "Content-Type: application/json" \
  -d '{
    "name": "OpenAI",
    "slug": "openai",
    "provider_type": "api_key",
    "description": "OpenAI API for GPT models",
    "api_key_instructions": "Visit https://platform.openai.com/api-keys to create a key",
    "api_key_url": "https://platform.openai.com/api-keys",
    "icon_url": "https://example.com/icons/openai.svg",
    "documentation_url": "https://platform.openai.com/docs"
  }'
```

**OAuth2 provider (e.g., Google AI):**

```bash
curl -X POST https://auth.example.com/api/v1/providers \
  -H "Authorization: Bearer <admin_access_token>" \
  -H "Content-Type: application/json" \
  -d '{
    "name": "Google AI",
    "slug": "google-ai",
    "provider_type": "oauth2",
    "authorization_url": "https://accounts.google.com/o/oauth2/v2/auth",
    "token_url": "https://oauth2.googleapis.com/token",
    "revocation_url": "https://oauth2.googleapis.com/revoke",
    "default_scopes": ["https://www.googleapis.com/auth/generative-language"],
    "client_id": "your-google-client-id",
    "client_secret": "your-google-client-secret",
    "supports_pkce": true
  }'
```

### Step 2: Configure Service Requirements

For each downstream service that needs provider tokens, add provider requirements:

```bash
curl -X POST https://auth.example.com/api/v1/services/<service_id>/requirements \
  -H "Authorization: Bearer <admin_access_token>" \
  -H "Content-Type: application/json" \
  -d '{
    "provider_config_id": "<openai_provider_id>",
    "required": true,
    "injection_method": "bearer"
  }'
```

### Step 3: Configure Identity Propagation (Optional)

Enable identity propagation on downstream services to forward user identity:

```bash
curl -X PUT https://auth.example.com/api/v1/services/<service_id> \
  -H "Authorization: Bearer <admin_access_token>" \
  -H "Content-Type: application/json" \
  -d '{
    "identity_propagation_mode": "both",
    "identity_include_user_id": true,
    "identity_include_email": true,
    "identity_include_name": true,
    "identity_jwt_audience": "https://my-service.example.com"
  }'
```

### Common Provider Configurations

| Provider   | Type    | Auth Model  | Notes                                    |
|------------|---------|-------------|------------------------------------------|
| OpenAI     | api_key | API key     | `Authorization: Bearer <key>` injection  |
| Anthropic  | api_key | API key     | `x-api-key: <key>` header injection      |
| Mistral    | api_key | API key     | `Authorization: Bearer <key>` injection  |
| Cohere     | api_key | API key     | `Authorization: Bearer <key>` injection  |
| Google AI  | oauth2  | OAuth2+PKCE | Requires Google Cloud OAuth credentials  |
| Azure OpenAI | api_key | API key  | `api-key: <key>` header injection        |

### OAuth2 Redirect URI

For OAuth2 providers, register the following redirect URI in the provider's developer console:

```
https://auth.example.com/api/v1/providers/callback
```

This is the generic callback endpoint that handles all provider OAuth flows.

---

## Telegram Bot Setup

The approval system uses a Telegram bot to send push notifications for access approval requests. This is optional -- the approval system works via the web UI without Telegram.

### 1. Create a Telegram Bot

1. Open Telegram and search for [@BotFather](https://t.me/BotFather)
2. Send `/newbot` and follow the prompts to name your bot
3. BotFather will give you a **bot token** (e.g. `123456789:ABCdef...`)
4. Note the **bot username** (e.g. `NyxIDBot`)

### 2. Configure Environment Variables

Add to your `.env` or environment:

```bash
TELEGRAM_BOT_TOKEN=123456789:ABCdefGHIjklMNO     # From BotFather
TELEGRAM_BOT_USERNAME=NyxIDBot                     # Without @
```

For **production (webhook mode)**, also set:

```bash
TELEGRAM_WEBHOOK_SECRET=your-random-secret-string  # Generate with: openssl rand -hex 32
TELEGRAM_WEBHOOK_URL=https://auth.nyxid.dev/api/v1/webhooks/telegram
```

The `TELEGRAM_WEBHOOK_URL` must be a publicly accessible HTTPS URL that Telegram can reach.

For **development (long polling mode)**, omit `TELEGRAM_WEBHOOK_URL` and `TELEGRAM_WEBHOOK_SECRET`. The backend will automatically fall back to `getUpdates` long polling -- no public URL or tunnel required.

### 3. Delivery Modes

NyxID supports two Telegram delivery modes, selected automatically based on environment configuration:

| Mode | When | How |
|------|------|-----|
| **Webhook** (production) | `TELEGRAM_WEBHOOK_URL` + `TELEGRAM_WEBHOOK_SECRET` are set | Backend calls `setWebhook` at startup. Telegram pushes updates to `POST /api/v1/webhooks/telegram`. |
| **Long polling** (development) | Only `TELEGRAM_BOT_TOKEN` is set | Backend calls `deleteWebhook` at startup, then polls `getUpdates` in a background task (30s timeout). |

Both modes share the same update processing logic and support all features (approval callbacks, account linking).

#### Webhook Mode Setup

Register the webhook URL with Telegram using the Bot API:

```bash
curl -X POST "https://api.telegram.org/bot${TELEGRAM_BOT_TOKEN}/setWebhook" \
  -H "Content-Type: application/json" \
  -d '{
    "url": "https://auth.nyxid.dev/api/v1/webhooks/telegram",
    "secret_token": "your-random-secret-string",
    "allowed_updates": ["callback_query", "message"]
  }'
```

Verify the webhook is set:

```bash
curl "https://api.telegram.org/bot${TELEGRAM_BOT_TOKEN}/getWebhookInfo"
```

> **Note:** The backend also registers the webhook at startup when the env vars are set. The manual curl command is useful for verification or re-registration.

#### Long Polling Mode Setup

No additional setup needed. Just set `TELEGRAM_BOT_TOKEN` (and optionally `TELEGRAM_BOT_USERNAME`) and start the backend. The polling loop starts automatically and logs:

```
INFO nyxid: Telegram polling mode: starting getUpdates loop
```

If a webhook was previously registered, the backend calls `deleteWebhook` first to avoid conflicts.

### 4. User Account Linking

Users link their Telegram account from the NyxID settings page:

1. User navigates to **Settings > Notifications** in the NyxID dashboard
2. Clicks **Link Telegram** to generate a one-time link code
3. Sends `/start NYXID-XXXXXXXX` to the bot in Telegram
4. The bot confirms the link and starts sending approval notifications

### 5. Development Setup

For local development, **long polling mode** is recommended (see above). Simply set `TELEGRAM_BOT_TOKEN` and omit the webhook variables.

If you prefer webhook mode locally, you need a public URL:

- **ngrok**: `ngrok http 3001` and use the generated HTTPS URL
- **Cloudflare Tunnel**: `cloudflared tunnel --url http://localhost:3001`

Set `TELEGRAM_WEBHOOK_URL` to the public URL + `/api/v1/webhooks/telegram`.

### 6. Approval Expiry Configuration

The background task that expires timed-out approval requests runs every `APPROVAL_EXPIRY_INTERVAL_SECS` seconds (default: 5). Adjust this if you need more or less frequent expiry checks:

```bash
APPROVAL_EXPIRY_INTERVAL_SECS=10  # Check every 10 seconds
```

---

## Security Hardening Checklist

### Before Going Live

- [ ] `ENVIRONMENT` is set to `production`
- [ ] `ENCRYPTION_KEY` is a unique, randomly generated 32-byte key (not the `.env.example` placeholder)
- [ ] RSA key pair is pre-generated and mounted read-only (not auto-generated)
- [ ] `BASE_URL` uses `https://` (enables Secure cookie flag)
- [ ] `FRONTEND_URL` uses `https://` (CORS origin)
- [ ] `DATABASE_URL` uses TLS (`?tls=true`)
- [ ] MongoDB authentication is enabled with a strong password
- [ ] If using Telegram approval in webhook mode: `TELEGRAM_WEBHOOK_SECRET` is a randomly generated string
- [ ] If using Telegram approval in webhook mode: `TELEGRAM_WEBHOOK_URL` uses `https://`
- [ ] If using Telegram approval in polling mode (dev only): `TELEGRAM_WEBHOOK_URL` is not set
- [ ] SMTP is configured for transactional email (verify-email, reset-password)
- [ ] Social login secrets are set (if using social login)
- [ ] TLS is terminated at the reverse proxy
- [ ] Reverse proxy sets `X-Forwarded-For` for accurate IP-based rate limiting
- [ ] `RUST_LOG` is set to `nyxid=info,tower_http=warn` (no debug output)
- [ ] Private key file permissions are `600`
- [ ] `.env` file is not accessible via the web server
- [ ] No development tools (Mailpit, debug endpoints) are exposed
- [ ] Firewall rules restrict MongoDB port to backend servers only
- [ ] Backup strategy is in place and tested

### Ongoing

- [ ] Monitor the `/health` endpoint
- [ ] Review audit logs periodically
- [ ] Rotate encryption keys through the ordered reader-first procedure and authenticated field inventory (see [SECURITY.md](SECURITY.md#key-rotation)). `/health` and zero observed fallback decrypts are operational signals, not retirement proof. Resolve old-format/provider/key dependencies and retained backups and rollback readers before retiring any key; cloud KMS internal-version uncertainty remains a blocker.
- [ ] Rotate RSA keys on a schedule
- [ ] Keep dependencies updated (`cargo update`, `npm update`)
- [ ] Subscribe to security advisories for critical dependencies (argon2, jsonwebtoken, rsa, aes-gcm)

---

## Troubleshooting

### Backend fails to start

**"ENCRYPTION_KEY must be set"**
- Ensure `ENCRYPTION_KEY` is defined in your `.env` or environment variables.

**"ENCRYPTION_KEY is all zeros"**
- Replace the placeholder key from `.env.example` with a real key: `openssl rand -hex 32`

**"Failed to connect to database"**
- Verify MongoDB is running: `mongosh --eval "db.adminCommand('ping')"`
- Check `DATABASE_URL` format and credentials
- Ensure network connectivity between the backend and MongoDB

**"Failed to load JWT keys"**
- In production, RSA keys must exist at the configured paths
- Check file permissions: `ls -la keys/`
- Verify the key format: `openssl rsa -in keys/private.pem -check`

### Authentication issues

**Cookies not being set**
- Ensure `BASE_URL` and `FRONTEND_URL` match your actual URLs
- Check CORS: the frontend origin must exactly match `FRONTEND_URL`
- In production, cookies require `Secure` flag (HTTPS only)

**JWT verification fails after key rotation**
- This is expected. All tokens signed with the old key are invalidated.
- Users need to re-authenticate.

**Rate limiting too aggressive**
- Increase `RATE_LIMIT_PER_SECOND` and `RATE_LIMIT_BURST`
- Inspect the shared rate-window rows in MongoDB to identify the exhausted key

### Database issues

**Slow queries**
- NyxID creates indexes automatically. Verify they exist: `db.users.getIndexes()`
- Check MongoDB logs for slow query warnings
- Consider adding the MongoDB `slowms` profiler

**Connection pool exhaustion**
- Increase `DATABASE_MAX_CONNECTIONS` (default: 10)
- Check for connection leaks in custom extensions

### Frontend issues

**Blank page after deployment**
- Ensure SPA fallback is configured (serve `index.html` for all routes)
- Check browser console for CORS errors
- Verify the API URL in the frontend build matches the backend URL


## MongoDB pool saturation

Budget `DATABASE_MAX_CONNECTIONS` per process and per MongoDB server. A starting
recommendation for bursty assistant workloads is 50 per replica: 50 primary
application sockets for one replica, 100 for two. Include deployment surge,
other clients, two monitoring sockets per server per replica, and the minimum
pools on other members. Keep the projected peak below 80% of the database's
per-node connection limit. Confirm the Atlas tier's actual limit before choosing
a deployment setting. A larger pool supplies headroom but cannot repair a lock
cycle. The current configuration default remains 10.

Change-stream getMore calls use pooled connections while waiting. NyxID has
assistant live, execution-enrollment and machine-access change-stream consumers.
The HTTP requests-per-second limit and per-owner turn limits do not bound the
number of concurrent operations across the process.

NyxID warns when a server pool stays fully occupied for at least one second and
when a completed or failed checkout took at least one second. The warnings share
a 30-second rate limit per server pool. They contain the configured maximum and
elapsed milliseconds, with no command contents or account identifiers. Full
occupancy is not proof of queued requests or deadlock; correlate it with MongoDB
transaction/operation and resource telemetry. It is deliberately not reported as
an exact queue length because the driver's CMAP events cannot track cancellation
of individual waiters.

Rust MongoDB driver 3.5.2 does not apply a checkout-wait deadline. Its parsed
`waitQueueTimeoutMS` is not connected to the checkout implementation, and
`socketTimeoutMS` is unsupported. Server-selection and connection-establishment
timeouts do not bound checkout. Do not rely on those settings to limit a saturated
pool's wait. A future checkout-only timeout needs driver support and a reviewed
retryable error contract; cancelling a write after dispatch can leave an unknown
outcome.

On normal replica sets, a transaction returns its connection between commands;
load-balanced topology pins a physical connection, and sharded transactions pin
a mongos address. Independently, a database operation waiting on an uncommitted
write occupies a connection. Enough such operations can prevent the writer's
COMMIT from acquiring a connection. Keep writes in the intended session and
avoid nested writers against a fence already held by their caller.

Ordinary chained audit appends serialize through one process-wide async gate,
before any database work. Queued callers hold no connection. They share a
120-second queue/retry budget, allowing the default 60-second server transaction
lifetime plus 60 seconds for cleanup and recovery; conflicts retry with capped jittered
backoff instead of an eight-attempt limit. This is a retry budget, not a general
MongoDB timeout: it does not cancel a dispatched command or an uncertain commit.
Existing shorter caller deadlines still apply. An awaited audit can delay its
request or background job; the existing error handling remains in place.

Each attempt still reads the tail and inserts in a short transaction: a plain
unique insert under the gate can strand COMMIT with pool one and can wait beyond
the retry deadline. Transactional conflicts release the socket so the holder
and retry clock can progress. A successful append adds one COMMIT command.
Caller-owned in-session appends do not take the process gate. Stored audit rows,
UUID replay checks and hash bytes do not change; mixed versions remain readable,
but old replicas retain the ordinary-insert starvation and eight-attempt loss
risks. Rollback restores those risks.

The queue is in memory. Deadline exhaustion, shorter caller timeouts or process
loss can still drop best-effort ordinary audits. Required security-effect
intents retain and retry their own payloads; chain verification cannot reconstruct
an ordinary audit entry that was never saved. Serialization has a throughput
ceiling set by database round-trip latency; compare observed audit volume with
measured append throughput before considering ordered batching.

An ordinary audit caller still waiting for the gate after one second emits a
warning with `queue_wait_ms` and `waiting` (queued callers only, excluding the
holder). The warning repeats at most once per 30 seconds per process across all
waiters, including while the holder remains blocked. Cancellation and queue
deadlines remove waiters from the count. No audit content is logged, and the
warning does no database work.

A 5 ms majority commit alone caps the serial appender below 200 entries/second
per replica; 10 ms caps it below 100. Tail reads, inserts, network latency and
conflict retries lower those ceilings further. Sustained arrivals above capacity
grow the in-memory queue; each caller has at most its remaining 120-second
queue/retry budget (or a shorter caller deadline) before failing. Best-effort
entries can then be lost. The warning exposes queue pressure; it does not add a
durable queue, admission limit or batching.
