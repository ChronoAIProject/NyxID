# Machine container seccomp profile

Based on the Apache-2.0 Moby default profile: https://raw.githubusercontent.com/moby/profiles/2ceae35d351c156cb5a8efc0fdc4a08cf94569d8/seccomp/default.json

Allows clone, unshare and setns for Chromium user-namespace sandboxing. All other Docker default syscall restrictions remain. No additional capabilities are granted. Agent workers set no-new-privileges before execution.
