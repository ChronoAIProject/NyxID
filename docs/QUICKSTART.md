# NyxID Self-Host Quickstart

Step-by-step manual setup for running NyxID on your own machine, plus troubleshooting, uninstall/reinstall, and post-install AI-agent wiring.

For the one-paragraph overview and the AI-assisted setup prompt (drive the whole flow from Claude Code / Cursor), see the [README Quick Start](../README.md#quick-start).

---

## Prerequisites

- [Docker](https://docs.docker.com/get-docker/) — required for the server stack (backend, frontend, MongoDB). ~2 GB disk for images on first pull.
- A bash-compatible terminal — macOS Terminal, Linux shell, or [WSL (Windows Subsystem for Linux)](https://learn.microsoft.com/en-us/windows/wsl/install) on Windows.
- [Rust / Cargo](https://www.rust-lang.org/tools/install) — **optional**, only needed if you install the `nyxid` CLI (see [Install the `nyxid` CLI](#optional-install-the-nyxid-cli) below). The installer will set this up automatically if missing. Budget ~1.5 GB disk (~300 MB for the toolchain plus ~1 GB for the build cache) and 3–10 minutes for the first compile.

Total disk footprint: ~2 GB for the server only, ~3.5 GB if you also install the CLI from source.

---

## Step 1 of 3 — Check your system

Paste this into your terminal:

```bash
bash << 'CHECK'
err=0
for cmd in git docker openssl curl; do
  if ! command -v "$cmd" >/dev/null 2>&1; then echo "Missing: $cmd"; err=1; fi
done
if ! docker compose version >/dev/null 2>&1; then echo "Missing: docker compose (v2 plugin)"; err=1; fi
if ! docker info >/dev/null 2>&1; then echo "Docker is not running. Start Docker Desktop and re-run."; err=1; fi
if [ "$err" -eq 1 ]; then exit 1; fi
echo "All good — proceed to Step 2."
CHECK
```

---

## Step 2 of 3 — Install and start

> **This is a first-time install.** If you already have NyxID set up locally, run `./scripts/uninstall.sh --yes` from inside `NyxID/` first (see [Uninstall & reinstall](#uninstall--reinstall) below), then come back here.

The block below is wrapped in `bash << 'INSTALL' ... INSTALL` so it runs under bash regardless of your outer shell — no `zsh: command not found: #` errors on macOS. The trailing `cd NyxID` runs in your interactive shell after the bash subshell exits, so you land inside the checkout for later commands (stop, uninstall, CLI install).

```bash
bash << 'INSTALL'
set -e

# ── Pre-flight: refuse to run on an existing install ──
# Checks for install STATE (.env.dev or any nyx-flavored Mongo volume), NOT
# the NyxID/ source tree — so re-running this block after ./scripts/uninstall.sh
# works cleanly. The volume grep matches nyxid_mongodb_data (default compose
# project), nyx_mongodb_data, or any other nyx*_mongodb_data variant from a
# renamed checkout, without false-positing on unrelated MongoDB projects.
if [ -f NyxID/.env.dev ] \
  || docker volume ls --format '{{.Name}}' 2>/dev/null | grep -qE 'nyx.*_mongodb_data$'; then
  echo "Existing NyxID install state detected."
  if [ -d NyxID ]; then
    echo "Uninstall first, then re-paste this block:"
    echo "    cd NyxID && ./scripts/uninstall.sh --yes && cd .."
  else
    echo "NyxID/ is gone but a stale MongoDB volume remains. Remove it, then re-paste:"
    echo "    docker volume ls --format '{{.Name}}' | grep -E 'nyx.*_mongodb_data\$' | xargs -r docker volume rm"
  fi
  exit 0
fi

# Clone only if the source tree isn't already here (post-uninstall reinstall
# reuses the existing checkout; uninstall.sh doesn't delete the repo itself).
[ -d NyxID ] || git clone https://github.com/ChronoAIProject/NyxID.git
cd NyxID

# ── Generate .env.dev (dev config) and link for Docker ──
EK=$(openssl rand -hex 32)
cat > .env.dev << EOF
MONGO_ROOT_PASSWORD=$(openssl rand -hex 24)
ENCRYPTION_KEY=$EK
BASE_URL=http://localhost:3001
FRONTEND_URL=http://localhost:3000
ENVIRONMENT=development
JWT_PRIVATE_KEY_PATH=/app/keys/private.pem
JWT_PUBLIC_KEY_PATH=/app/keys/public.pem
INVITE_CODE_REQUIRED=false
AUTO_VERIFY_EMAIL=true
EMAIL_AUTH_ENABLED=true
RUST_LOG=nyxid=info,tower_http=info
EOF
ln -sf .env.dev .env.production

# ── Generate signing keys (LibreSSL fallback for macOS) ──
mkdir -p keys
openssl genrsa -out keys/private.pem 4096 2>/dev/null
openssl rsa -in keys/private.pem -RSAPublicKey_out -out keys/public.pem 2>/dev/null \
  || openssl rsa -in keys/private.pem -pubout -out keys/public.pem 2>/dev/null

# ── Pull images and start the stack ──
echo "Downloading NyxID (this may take a few minutes on first run)..."
docker compose -f docker-compose.yml -f docker-compose.prod.yml \
  --env-file .env.production pull
docker compose -f docker-compose.yml -f docker-compose.prod.yml \
  --env-file .env.production up -d

# ── Wait for the server (up to 90s) ──
# Track success explicitly so we print EXACTLY ONE of the two outcome
# messages below, never both. Fixes #282 where timeout + success printed
# together when /health didn't come up in time.
echo "Waiting for NyxID to start..."
ok=0
n=0
while [ "$n" -lt 45 ]; do
  if curl -sf http://localhost:3001/health >/dev/null 2>&1; then
    ok=1
    break
  fi
  n=$((n+1))
  sleep 2
done

if [ "$ok" -eq 1 ]; then
  echo ""
  echo "✓ NyxID is running at http://localhost:3000"
  echo "  Save your encryption key (needed if you reset the database): $EK"
else
  echo ""
  echo "✗ Timed out waiting for NyxID to start."
  echo "  Check logs:  docker logs nyxid-backend"
  echo "  Reset state: see the 'Uninstall & reinstall' section below"
fi
INSTALL

cd NyxID 2>/dev/null || true
```

---

## Step 3 of 3 — Register and connect

1. Open `http://localhost:3000` in your browser
2. Register with your name, email, and a password — no email verification needed (accounts are auto-verified in dev mode)
3. Log in and connect your AI agent using one of the methods in [Connect your AI tool](#connect-your-ai-tool) below

To stop NyxID: `docker compose -f docker-compose.yml -f docker-compose.prod.yml down`

---

## Optional: Install the `nyxid` CLI

The server stack above is fully usable from the web console — the CLI (Command Line Interface) is only needed if you want to script credential setup, manage credential nodes, or drive NyxID from your terminal. Skip this section if you'd rather stay in the browser.

> **Heads-up:** the installer builds from source via Cargo. It will install Rust automatically if you don't already have it (~300 MB) and then compile the CLI (~1 GB build cache, 3–10 minutes on first run). Make sure you have ~1.5 GB free.

```bash
bash -c "$(curl -fsSL https://raw.githubusercontent.com/ChronoAIProject/NyxID/main/skills/nyxid/tools/install.sh)"
source ~/.cargo/env                               # make nyxid available in current shell
nyxid --version                                   # verify
```

> Already have Rust? You can also install with: `cargo install --git https://github.com/ChronoAIProject/NyxID.git nyxid-cli`

---

## Connect your AI tool

Once your stack is up and you've registered an account, the next step is to connect a real downstream AI Service (OpenAI, Anthropic, GitHub, etc.) and verify the proxy actually works. **Wiring MCP alone won't show real tools** — until a real service is connected and verified, your AI agent will only see NyxID's `nyx__...` meta-tools and you'll wonder why nothing's working.

The full flow lives in **[docs/CONNECTING_SERVICES.md](CONNECTING_SERVICES.md)**. It's base-URL-agnostic and covers all four paths (AI-driven, CLI, web console, direct API). Use `http://localhost:3001` as your `<BASE_URL>` for self-host.

---

## Uninstall & reinstall

Quickstart is a **first-time install**. To reinstall — e.g. to try a new config, wipe test data, or recover from a broken state — uninstall first, then re-run [Step 2](#step-2-of-3--install-and-start).

```bash
cd NyxID
./scripts/uninstall.sh               # interactive: type "wipe" to confirm
./scripts/uninstall.sh --yes         # non-interactive (CI / repeat testing)
./scripts/uninstall.sh --keep-config # keep .env.dev and keys/*.pem across reinstall
```

By default this removes:

- Docker containers (`nyxid-mongodb`, `nyxid-mailpit`, `nyxid-backend`, `nyxid-frontend`)
- The MongoDB named volume (`nyxid_mongodb_data`) — all NyxID accounts, encrypted credentials, and audit log entries
- `.env.dev`, `.env.production`, `keys/private.pem`, `keys/public.pem`

Docker images are preserved (no re-download). Pass `--keep-config` if you want to preserve your existing `ENCRYPTION_KEY` across reinstall (e.g. to keep encrypted database backups readable).

After uninstall, `cd ..` out of `NyxID/` and re-paste **Step 2** above. The pre-flight now checks for install *state* (`.env.dev` or the Mongo volume), not the source tree — so the existing `NyxID/` checkout is reused and only regeneration runs.

### Recovering an orphan volume

If you hit issue [#280](https://github.com/ChronoAIProject/NyxID/issues/280) on an older quickstart and manually deleted your `NyxID/` checkout, but a stale MongoDB volume survived, you don't need to re-clone just to run `uninstall.sh`. Remove any nyx-flavored volume directly — this matches `nyxid_mongodb_data` (default), `nyx_mongodb_data`, or any `nyx*_mongodb_data` variant from a renamed checkout:

```bash
docker volume ls --format '{{.Name}}' | grep -E 'nyx.*_mongodb_data$' | xargs -r docker volume rm
docker rm -f nyxid-mongodb nyxid-mailpit nyxid-backend nyxid-frontend 2>/dev/null || true
```

Then re-paste Step 2 — the pre-flight will pass and Step 2 will clone fresh.

### Stuck on SCRAM failure?

If `docker logs nyxid-backend` shows `SCRAM failure: Authentication failed`, your MongoDB volume still has the previous `MONGO_ROOT_PASSWORD` baked in from a prior run, and `.env.dev` no longer matches. Run `./scripts/uninstall.sh --yes` to wipe the volume, then re-run [Step 2](#step-2-of-3--install-and-start). See [#280](https://github.com/ChronoAIProject/NyxID/issues/280).

---

## Production deployment

For production deployment (TLS (Transport Layer Security), custom domain, email verification), see [DEPLOYMENT.md](DEPLOYMENT.md).
