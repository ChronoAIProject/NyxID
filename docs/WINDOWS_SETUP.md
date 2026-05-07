# Windows Setup

NyxID's bash quickstart, `docker`, `nyxid`, and `curl` examples assume a Unix-compatible shell. Install WSL once, then run every command in the rest of the docs from your Ubuntu shell:

1. Open PowerShell as Administrator and run `wsl --install`. Restart when prompted.
2. Install [Docker Desktop](https://docs.docker.com/desktop/install/windows-install/), then enable WSL integration: **Settings → Resources → WSL Integration → toggle on for your distro**.
3. Launch Ubuntu (or your installed distro). Clone and work **inside the WSL filesystem** (e.g. `~/NyxID`) — avoid `/mnt/c/...` for I/O speed and to skip permission warnings during key generation.

After that, every bash, `docker`, `nyxid`, and `curl` example in NyxID's docs runs unchanged.

> **Can't use WSL?** If your Windows version doesn't support WSL2 (Windows 10 before version 2004) or your IT policy blocks it, [Git Bash](https://gitforwindows.org/) also runs the bash quickstart commands. Docker Desktop is still required.
