param(
    [switch]$FromSource
)

$ErrorActionPreference = "Stop"

$RepoSlug = "ChronoAIProject/NyxID"
$RepoUrl = "https://github.com/$RepoSlug.git"
$CosignIssuer = "https://token.actions.githubusercontent.com"
$LocalBin = Join-Path $HOME ".local\bin"
$CargoBin = Join-Path $HOME ".cargo\bin"

function Write-Info($Message) {
    Write-Host "  $Message"
}

function Write-Warn($Message) {
    Write-Warning $Message
}

function Fail($Message) {
    throw $Message
}

function Get-TargetTriple {
    $arch = [System.Runtime.InteropServices.RuntimeInformation]::OSArchitecture
    switch ($arch.ToString()) {
        "X64" { return "x86_64-pc-windows-msvc" }
        "Arm64" { return "aarch64-pc-windows-msvc" }
        default { return $null }
    }
}

function Get-LatestTag {
    $headers = @{ "User-Agent" = "nyxid-installer" }
    return (Invoke-RestMethod -Headers $headers -Uri "https://api.github.com/repos/$RepoSlug/releases/latest").tag_name
}

function Download-File($Url, $Destination) {
    Invoke-WebRequest -Uri $Url -OutFile $Destination
}

function Ensure-UserPath($PathEntry) {
    $current = [Environment]::GetEnvironmentVariable("Path", "User")
    $entries = @()
    if ($current) {
        $entries = $current -split ';'
    }

    if ($entries -contains $PathEntry) {
        return
    }

    $updated = @($PathEntry) + $entries
    [Environment]::SetEnvironmentVariable("Path", ($updated -join ';'), "User")
    Write-Info "Added $PathEntry to the user PATH. Open a new terminal to pick it up."
}

function Install-FromSource {
    if (-not (Get-Command cargo -ErrorAction SilentlyContinue)) {
        Fail "cargo was not found. Install Rust from https://rustup.rs/ and re-run with --FromSource."
    }

    Write-Info "Installing NyxID CLI from source..."
    & cargo install --git $RepoUrl nyxid-cli --force --locked
    if ($LASTEXITCODE -ne 0) {
        Fail "cargo install failed."
    }

    New-Item -ItemType Directory -Force -Path $LocalBin | Out-Null
    Copy-Item (Join-Path $CargoBin "nyxid.exe") (Join-Path $LocalBin "nyxid.exe") -Force
    Write-Info "Installed NyxID CLI at $LocalBin\nyxid.exe"
}

function Install-Prebuilt {
    $target = Get-TargetTriple
    if (-not $target) {
        Fail "No prebuilt NyxID binary is published for this Windows architecture. Re-run with -FromSource."
    }

    $tag = Get-LatestTag
    $version = $tag.TrimStart('v')
    $archiveName = "nyxid-$version-$target.zip"
    $releaseBase = "https://github.com/$RepoSlug/releases/download/$tag"
    $tmpDir = Join-Path ([System.IO.Path]::GetTempPath()) ("nyxid-install-" + [guid]::NewGuid().ToString("N"))

    New-Item -ItemType Directory -Force -Path $tmpDir | Out-Null
    try {
        $archivePath = Join-Path $tmpDir $archiveName
        $checksumsPath = Join-Path $tmpDir "SHA256SUMS"
        $signaturePath = Join-Path $tmpDir "SHA256SUMS.sig"
        $certPath = Join-Path $tmpDir "SHA256SUMS.pem"

        Write-Info "Downloading NyxID CLI $version for $target..."
        Download-File "$releaseBase/$archiveName" $archivePath
        Download-File "$releaseBase/SHA256SUMS" $checksumsPath
        Download-File "$releaseBase/SHA256SUMS.sig" $signaturePath
        Download-File "$releaseBase/SHA256SUMS.pem" $certPath

        if (Get-Command cosign -ErrorAction SilentlyContinue) {
            Write-Info "Verifying SHA256SUMS signature with cosign..."
            & cosign verify-blob `
                --certificate-identity "https://github.com/$RepoSlug/.github/workflows/release.yml@refs/tags/$tag" `
                --certificate-oidc-issuer $CosignIssuer `
                --certificate $certPath `
                --signature $signaturePath `
                $checksumsPath | Out-Null
            if ($LASTEXITCODE -ne 0) {
                Fail "cosign verification failed for SHA256SUMS."
            }
        } else {
            Write-Warn "cosign not found; skipping SHA256SUMS signature verification."
        }

        $checksumLine = Select-String -Path $checksumsPath -Pattern ([regex]::Escape($archiveName) + '$') | Select-Object -First 1
        if (-not $checksumLine) {
            Fail "SHA256SUMS does not contain an entry for $archiveName."
        }

        $expectedHash = ($checksumLine.Line -split '\s+')[0].ToLowerInvariant()
        $actualHash = (Get-FileHash -Algorithm SHA256 $archivePath).Hash.ToLowerInvariant()
        if ($expectedHash -ne $actualHash) {
            Fail "Archive checksum mismatch for $archiveName."
        }

        $extractDir = Join-Path $tmpDir "extract"
        Expand-Archive -Path $archivePath -DestinationPath $extractDir -Force
        $binary = Get-ChildItem -Path $extractDir -Recurse -Filter "nyxid.exe" | Select-Object -First 1
        if (-not $binary) {
            Fail "Downloaded archive did not contain nyxid.exe."
        }

        New-Item -ItemType Directory -Force -Path $LocalBin | Out-Null
        Copy-Item $binary.FullName (Join-Path $LocalBin "nyxid.exe") -Force
        Write-Info "Installed NyxID CLI at $LocalBin\nyxid.exe"

        if (Test-Path $CargoBin) {
            Copy-Item (Join-Path $LocalBin "nyxid.exe") (Join-Path $CargoBin "nyxid.exe") -Force
            Write-Info "Copied nyxid.exe into $CargoBin for existing Cargo users."
        }
    }
    finally {
        Remove-Item -Recurse -Force $tmpDir -ErrorAction SilentlyContinue
    }
}

if ($FromSource) {
    Install-FromSource
} else {
    Install-Prebuilt
}

Ensure-UserPath $LocalBin

$installed = Join-Path $LocalBin "nyxid.exe"
if (Test-Path $installed) {
    Write-Info "Verified: $(& $installed --version)"
    Write-Info ""
    Write-Info "Installation complete!"
} else {
    Fail "nyxid.exe was not found after installation."
}
