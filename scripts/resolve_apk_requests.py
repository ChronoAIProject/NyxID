"""Explicit distro additions for the reviewed APK-installing stages."""
REQUESTS = {
    "frontend-runtime": ("frontend/Dockerfile", "runtime", ["python3"]),
    "updater-builder": ("cli/Dockerfile.machine-updater", "builder",
                        ["build-base", "cmake", "clang", "lld", "perl", "linux-headers", "python3"]),
    "updater-e2e": ("cli/Dockerfile.machine-updater", "e2e", ["nodejs", "npm", "openssl"]),
}
