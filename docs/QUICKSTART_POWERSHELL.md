# NyxID Self-Host Quickstart (PowerShell)

Step-by-step manual setup for running NyxID on Windows using native PowerShell, plus troubleshooting, uninstall/reinstall, and post-install AI-agent wiring.

> **Looking for the bash version?** macOS, Linux, WSL, and Git Bash users should follow **[QUICKSTART_BASH.md](QUICKSTART_BASH.md)** — it is the default and is kept in lockstep with the AI-assisted setup prompt in the README.

For the one-paragraph overview and the AI-assisted setup prompt (drive the whole flow from Claude Code / Cursor), see the [README Quick Start](../README.md#quick-start).

---

## Prerequisites

- **PowerShell 7+** — required. `winget install Microsoft.PowerShell`. Windows 10/11 ships PowerShell 5.1, but several commands below rely on .NET Core APIs that are only available in PowerShell 7+. Verify with `$PSVersionTable.PSVersion`. *Have bash available (Git Bash, WSL, macOS, Linux)? Use [QUICKSTART_BASH.md](QUICKSTART_BASH.md) instead — it's simpler and is kept in lockstep with the AI-assisted setup prompt in the README.*
- **[Docker Desktop](https://docs.docker.com/desktop/install/windows-install/)** — required for the server stack (backend, frontend, MongoDB). ~2 GB disk for images on first pull. Make sure Docker Desktop is running before Step 1.
- **OpenSSL** — required for the encryption key and JWT signing keys. Install via:
  ```powershell
  winget install ShiningLight.OpenSSL.Light
  ```
  After installation, restart PowerShell so `openssl.exe` is on `PATH`. Alternatively, if you have Git for Windows installed, use its bundled binary at `C:\Program Files\Git\usr\bin\openssl.exe`.
- **`curl.exe`** — ships with Windows 10/11. PowerShell aliases `curl` to `Invoke-WebRequest`, so the snippets below explicitly call `curl.exe` to avoid the alias.
- **[Rust / Cargo](https://www.rust-lang.org/tools/install)** — **optional**, only needed if you install the `nyxid` CLI (see [Install the `nyxid` CLI](#optional-install-the-nyxid-cli) below). Budget ~1.5 GB disk and 3–10 minutes for the first compile.

Total disk footprint: ~2 GB for the server only, ~3.5 GB if you also install the CLI from source.

> **Symlink note:** Step 2 creates `.env.production` as a copy of `.env.dev` rather than a symlink. PowerShell's `New-Item -ItemType SymbolicLink` requires either administrator rights or Windows Developer Mode enabled, so the bash version's `ln -sf` is replaced with `Copy-Item` for friction-free setup. If you later edit `.env.dev`, copy it over `.env.production` again.

---

## Step 1 of 3 — Check your system

Paste this into PowerShell:

```powershell
$err = $false
foreach ($cmd in @('git', 'docker', 'openssl', 'curl.exe')) {
  if (-not (Get-Command $cmd -ErrorAction SilentlyContinue)) {
    Write-Host "Missing: $cmd"
    $err = $true
  }
}
try { docker compose version | Out-Null } catch { Write-Host "Missing: docker compose (v2 plugin)"; $err = $true }
try { docker info 2>&1 | Out-Null; if ($LASTEXITCODE -ne 0) { throw } } catch { Write-Host "Docker is not running. Start Docker Desktop and re-run."; $err = $true }
if ($err) { exit 1 }
Write-Host "All good - proceed to Step 2."
```

---

## Step 2 of 3 — Install and start

> **This is a first-time install.** If you already have NyxID set up locally, run `.\scripts\uninstall.sh --yes` from inside `NyxID/` first via Git Bash or WSL (see [Uninstall & reinstall](#uninstall--reinstall) below), then come back here. PowerShell does not run `.sh` scripts directly.

> **If you've run NyxID before:** a stale MongoDB volume can keep the old `MONGO_ROOT_PASSWORD` even after `.env.dev` is regenerated, which shows up as `SCRAM failure: Authentication failed` in `docker logs nyxid-backend`. To intentionally wipe local test data before re-running with a fresh password, run this from the existing `NyxID/` checkout:
>
> ```powershell
> docker compose -f docker-compose.yml -f docker-compose.prod.yml down -v
> ```
>
> This deletes local NyxID accounts, encrypted credentials, and audit logs. It does not change the quickstart automatically because deleting user data must be explicit.

The block below checks install state, reuses an existing checkout when safe, generates development env files and signing keys, starts Docker, and waits for `/health`.

```powershell
$ErrorActionPreference = 'Stop'

# Detect existing install state
$volumes = docker volume ls --format '{{.Name}}' 2>$null
$staleVolume = $volumes | Where-Object { $_ -match '^nyx.*_mongodb_data$' }
if ((Test-Path 'NyxID/.env.dev') -or $staleVolume) {
  Write-Host "Existing NyxID install state detected."
  if (Test-Path 'NyxID') {
    Write-Host "Uninstall first via Git Bash, then re-paste this block:"
    Write-Host "    cd NyxID; bash ./scripts/uninstall.sh --yes; cd .."
  } else {
    Write-Host "NyxID/ is gone but a stale MongoDB volume remains. Remove it, then re-paste:"
    Write-Host @'
    docker volume ls --format '{{.Name}}' | Select-String 'nyx.*_mongodb_data$' | ForEach-Object { docker volume rm $_.ToString().Trim() }
'@
  }
  return
}

if (-not (Test-Path 'NyxID')) {
  git clone https://github.com/ChronoAIProject/NyxID.git
}
Set-Location NyxID

$EK = (openssl rand -hex 32).Trim()
$MONGO_PASS = (openssl rand -hex 24).Trim()

@"
MONGO_ROOT_PASSWORD=$MONGO_PASS
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
"@ | Set-Content -Path .env.dev -NoNewline

# Windows symlink requires admin / Developer Mode; copy instead
Copy-Item .env.dev .env.production -Force

New-Item -ItemType Directory -Force -Path keys | Out-Null
openssl genrsa -out keys/private.pem 4096 2>$null
# PKCS#1 with LibreSSL fallback to PKCS#8
openssl rsa -in keys/private.pem -RSAPublicKey_out -out keys/public.pem 2>$null
if ($LASTEXITCODE -ne 0) {
  openssl rsa -in keys/private.pem -pubout -out keys/public.pem 2>$null
}

Write-Host "Downloading NyxID (this may take a few minutes on first run)..."
docker compose -f docker-compose.yml -f docker-compose.prod.yml --env-file .env.production pull
docker compose -f docker-compose.yml -f docker-compose.prod.yml --env-file .env.production up -d

Write-Host "Waiting for NyxID to start..."
$ok = $false
for ($n = 0; $n -lt 45; $n++) {
  try {
    $resp = Invoke-WebRequest -Uri 'http://localhost:3001/health' -UseBasicParsing -TimeoutSec 2 -ErrorAction Stop
    if ($resp.StatusCode -eq 200) { $ok = $true; break }
  } catch { }
  Start-Sleep -Seconds 2
}

if ($ok) {
  Write-Host ""
  Write-Host "OK: NyxID is running at http://localhost:3000"
  Write-Host "  Save your encryption key (needed if you reset the database): $EK"
} else {
  Write-Host ""
  Write-Host "ERROR: Timed out waiting for NyxID to start."
  Write-Host "  Check logs:  docker logs nyxid-backend"
  Write-Host "  Reset state: see the 'Uninstall & reinstall' section below"
}
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

> **Heads-up:** the bash installer script (`install.sh`) is the recommended path on macOS / Linux but does not run natively in PowerShell. Use one of these alternatives:

**Option A — Git Bash (one-liner):** if you have Git for Windows, open Git Bash and run:

```bash
bash -c "$(curl -fsSL https://raw.githubusercontent.com/ChronoAIProject/NyxID/main/skills/nyxid/scripts/install.sh)"
```

**Option B — Cargo (PowerShell-native):** install [Rust](https://www.rust-lang.org/tools/install) first, then:

```powershell
cargo install --git https://github.com/ChronoAIProject/NyxID.git nyxid-cli --locked
nyxid --version
```

> **Note:** Some `nyxid node daemon` subcommands (`install`, `start`, `stop`, `restart`, `status`, `logs`, `uninstall`) only support macOS launchd and Linux systemd. On Windows, run `nyxid node start` in the foreground or use `nyxid node docker` instead. See [Section 6 of CLAUDE.md](../CLAUDE.md#6-node-proxy-conventions) for the supported subcommand set.

---

## Connect your AI tool

Once your stack is up and you've registered an account, the next step is to connect a real downstream AI Service (OpenAI, Anthropic, GitHub, etc.) and verify the proxy actually works. **Wiring MCP alone won't show real tools** — until a real service is connected and verified, your AI agent will only see NyxID's `nyx__...` meta-tools and you'll wonder why nothing's working.

The full flow lives in **[docs/connecting-services/](connecting-services/)**. Start with the [Web UI walkthrough](connecting-services/web-ui.md) if it's your first service; for CLI / AI-driven (MCP) / Direct API, the [hub](connecting-services/README.md) links to one walkthrough per path. Use `http://localhost:3001` as your `<BASE_URL>` for self-host.

---

## Uninstall & reinstall

Quickstart is a **first-time install**. To reinstall — e.g. to try a new config, wipe test data, or recover from a broken state — uninstall first, then re-run [Step 2](#step-2-of-3--install-and-start).

The shipped uninstall scripts are bash. From PowerShell, invoke them via Git Bash or WSL:

```powershell
Set-Location NyxID
bash ./scripts/uninstall.sh         # interactive
bash ./scripts/uninstall.sh --yes   # non-interactive (CI / repeat testing)
bash ./scripts/uninstall.sh --keep-config  # keep .env.dev and keys/
```

If Git Bash / WSL is not available, run the equivalent steps directly in PowerShell:

```powershell
Set-Location NyxID
docker compose -f docker-compose.yml -f docker-compose.prod.yml down -v
docker rm -f nyxid-mongodb nyxid-mailpit nyxid-backend nyxid-frontend 2>$null
Remove-Item -Force -ErrorAction SilentlyContinue .env.dev, .env.production, keys/private.pem, keys/public.pem
```

By default this removes:

- Docker containers (`nyxid-mongodb`, `nyxid-mailpit`, `nyxid-backend`, `nyxid-frontend`)
- The MongoDB named volume (`nyxid_mongodb_data`) — all NyxID accounts, encrypted credentials, and audit log entries
- `.env.dev`, `.env.production`, `keys/private.pem`, `keys/public.pem`

Docker images are preserved (no re-download). Skip the `Remove-Item` line if you want to preserve your existing `ENCRYPTION_KEY` across reinstall (e.g. to keep encrypted database backups readable).

After uninstall, `Set-Location ..` out of `NyxID/` and re-paste **Step 2** above. The pre-flight now checks for install *state* (`.env.dev` or the Mongo volume), not the source tree — so the existing `NyxID/` checkout is reused and only regeneration runs.

### Recovering an orphan volume

If you hit issue [#280](https://github.com/ChronoAIProject/NyxID/issues/280) on an older quickstart and manually deleted your `NyxID/` checkout, but a stale MongoDB volume survived, you don't need to re-clone just to run `uninstall.sh`. Remove any nyx-flavored volume directly — this matches `nyxid_mongodb_data` (default), `nyx_mongodb_data`, or any `nyx*_mongodb_data` variant from a renamed checkout:

```powershell
docker volume ls --format '{{.Name}}' |
  Select-String 'nyx.*_mongodb_data$' |
  ForEach-Object { docker volume rm $_.ToString().Trim() }
docker rm -f nyxid-mongodb nyxid-mailpit nyxid-backend nyxid-frontend 2>$null
```

Then re-paste Step 2 — the pre-flight will pass and Step 2 will clone fresh.

### Stuck on SCRAM failure?

If `docker logs nyxid-backend` shows `SCRAM failure: Authentication failed`, your MongoDB volume still has the previous `MONGO_ROOT_PASSWORD` baked in from a prior run, and `.env.dev` no longer matches. Run the PowerShell uninstall block above to wipe the volume, then re-run [Step 2](#step-2-of-3--install-and-start).

## Done when...

- `curl.exe -sf http://localhost:3001/health` returns 200 (or `Invoke-WebRequest http://localhost:3001/health` returns 200).
- `http://localhost:3000` loads in your browser.
- You can register a user and log in.

---

## Production deployment

For production deployment (TLS (Transport Layer Security), custom domain, email verification), see [DEPLOYMENT.md](DEPLOYMENT.md). The deployment guide assumes a Linux host; PowerShell is for local self-host only.
