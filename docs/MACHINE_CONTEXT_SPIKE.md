# M1.3 Linux workspace feasibility spike

**Acceptance gate: failed. Stop before full-feature implementation.**
Tested 2026-10-04 on `feat/machine-contexts`, based on M1.2 `79e0f1c7`.

The acceptance contract is [MACHINE_AGENT_ISOLATION.md §3.1 and §7](MACHINE_AGENT_ISOLATION.md).
No production runtime, grants, protocol, UI, defaults or versions changed.
`shared_legacy` remains the default and `assistant:machine-contexts` remains default
off. The experiment is not a shipped or advertised isolated mode.

## Blocking findings

1. **Outside metadata writes remain possible.** On both tested kernels, a confined
   context successfully ran `os.utime(legacy_file, None)` against another UID's
   world-writable legacy file outside every allowed directory. Its mtime changed,
   although opening it for read, write or truncation was denied. This contradicts
   “writable only in context directories” and does not depend on UID reuse. A
   second control confirmed `chmod` succeeds on an outside file owned by the same
   UID. Fresh, never-reused UIDs reduce that second case but do not fix the first.
   Landlock documents these metadata operations as unhandled. Content protection
   must not be reported as a complete outside-write boundary.
2. **Private TMPDIR does not provide private POSIX shared memory.** Python
   `multiprocessing.Pool(2)` fails with `PermissionError(EACCES)` in `SemLock` on
   Python 3.12 in the container and Python 3.13 in the native VM. The identical
   UID/NNP/seccomp control without Landlock successfully creates a semaphore at
   `/dev/shm/sem.*`, despite the private TMPDIR. Granting the shared `/dev/shm`
   would violate the proposed allowlist; it was not granted. This affects ordinary
   Python workloads using process pools.

These need a revised enforcement mechanism. Landlock does not create private
mounts or IPC namespaces. A future design must mediate metadata changes and
provide private shared-memory semantics while preserving the official container's
default capabilities and AppArmor constraints. Blanket metadata-syscall denial
also needs tooling compatibility proof; it is not an implemented solution here.

Keep recommending separate existing machine containers/VMs for stronger separation.
Revisiting a privileged host provisioner/per-context containers would reopen the
explicitly deferred M1.5 decision. Do not repurpose the updater's Docker socket
authority. No full M1.3 implementation started after the failed gate.

## Prototype

The standalone test crate under `cli/tests/machine_context_spike` is deliberately
outside the product workspace. It reuses the production `Identity::prepare_agent`
implementation, including UID/GID/group handling, environment clearing, NNP and
the namespace-denying seccomp filter. It adds descriptor-pinned private context
directories, a Landlock filesystem allowlist, signal/abstract-socket scoping, and
`close_range(CLOSE_RANGE_CLOEXEC)` before exec. Its explicitly named negative
control modes are test-only and are not exposed by the node.

The prototype uses distinct UIDs 21001 and 21002 plus a legacy UID. It walks
root-owned, non-writable ancestors using `openat(O_PATH|O_DIRECTORY|O_NOFOLLOW)`;
workspace/home/tmp must actually belong to the context UID with mode 0700. It pins
cwd by descriptor, clears supplementary groups, and uses umask 0077. Wrong modes,
ownership or a symlink substitution refuse admission.

Landlock handles all filesystem rights through ABI 5, including `REFER`,
`TRUNCATE` and `IOCTL_DEV`; context device creation is not granted. ABI 6 signal
and abstract-socket scopes are enabled. Writable regular-file trees are limited
to context directories. Basic null/zero/random devices are accessible. Read-only
runtime grants cover binaries, libraries, headers, selected language resources
and individual public configuration files. Python wheels, distro metadata,
OpenSSL configuration and MIME types were needed by common tools. There is no
broad `/etc`, `/home`, `/var`, `/tmp`, `/workspace`, `/proc` or `/sys` grant.
Only CPU/memory/topology metadata files are allowed under proc/sys.

HOME, TMPDIR, XDG config/cache and Cargo home point into the context. Descriptors
>=3 receive `CLOSE_RANGE_CLOEXEC`, preserving Rust's spawn-error pipe until exec
and closing inherited descriptors at exec. Landlock alone does not retroactively
remove access from an already-open descriptor. The fixture requires the explicit
`--disposable` option because it creates users and changes synthetic legacy roots.

## Minimum ABI and host results

**ABI 6 / Linux 6.12 is necessary for this prototype, but is not sufficient for
the approved guarantee.** ABI 2 added refer/link control, ABI 3 truncation, ABI 5
device ioctl control, and ABI 6 signal/abstract-socket scopes. The cross-UID
`SIGCONT` control demonstrates why UID separation alone is insufficient while
children share a session. Production currently creates a process group, not a
new session.

| Environment | Kernel / ABI | Outcome |
|---|---|---|
| Official machine 0.56.0, native arm64 Docker Desktop, shipped seccomp | `7.0.14-linuxkit`, ABI 8 | 40 checks passed; metadata write allowed; Python Pool failed; gate failed |
| Same image, outer mount/umount2/pivot_root denial | Same kernel / ABI | Same results; no mounts, added capabilities, privileged mode or AppArmor bypass |
| Debian 13 arm64 VM, native separate-users profile layout | `6.12.111+deb13-cloud-arm64`, ABI 6 | 32 isolation/control checks passed; metadata write allowed; separate Pool probe failed; gate failed |

The official image was pinned to
`ghcr.io/chronoaiproject/nyxid/nyxid-node-machine@sha256:2f1fe82573fa86f99c3cca219be33ab846f17a7374cc40bcf51e2a56a1b4f7d4`.
Only the test launcher and distro Cargo/pip/venv packages were added for the spike.
No owner's container or volumes were used.

The Debian VM booted an independent kernel under QEMU/TCG. Both Landlock and
AppArmor were active in its LSM list. The native test uses the root-owned layout
of `--separate-users` and directly exercises production pre-exec handling; it does
not claim pairing, service installation or daemon-lifecycle coverage. The full
tool matrix ran in the container. Native testing stopped at the adversarial gate
plus a direct Python Pool counterexample. Native amd64, Docker's actual
`docker-default` AppArmor profile, browser contexts and full-feature lifecycle
validation remain untested. There is no shipping acceptance claim.

Probe the kernel and enabled LSMs, not the distribution/image version. Ubuntu
24.04's original 6.8 kernel (ABI 4) and Debian 12's original 6.1 kernel (ABI 2)
would not meet this prototype's minimum. HWE/backports require a live probe.
A container cannot supply Landlock independently of its host. Native macOS and
single-user Linux remain outside the approved scope. Injected Landlock
`EOPNOTSUPP` produced exit 125 without executing the child: unavailable enforcement
must fail closed, while shared legacy remains independently available.

## Adversarial evidence

| Check | Observed behavior |
|---|---|
| Sibling roots, including world-readable variants; legacy/config/browser fixture contents | Read/write/truncate denied; outside contents preserved |
| Absolute/relative traversal, outside cwd, `/proc/self/root` | Content access denied |
| Symlink chains, hardlinks, rename, parent replacement | Escapes denied; within-context links/renames work |
| Inherited files/directories, openat, `/proc/self/fd` | Closed at exec; outside access denied |
| Control without descriptor cleanup | Inherited outside descriptor remains readable despite Landlock |
| Sibling/legacy/same-UID-other-domain `/proc/PID/{mem,environ,fd}`, exact fd links, ptrace | Denied |
| SIGTERM and same-session cross-UID SIGCONT | Denied across contexts; own descendants remain controllable |
| Control without Landlock | Cross-UID SIGCONT succeeds |
| Shared `/tmp` and `/dev/shm` files/listings/creation | Denied; private tempfiles work |
| Widened context mode or substituted directory symlink | Refused before execution |
| Outside world-writable file mtime | **Changed: blocking counterexample** |
| Outside same-UID file mode | **Changed: additional limitation** |

Totals include negative controls that deliberately demonstrate inadequate
boundaries. They are not counts of distinct security properties. The fixture
prints `acceptance_gate: blocked` and exits **3** even when all expected denials
and controls run as designed. Unexpected fixture failures exit 1.

Abstract sockets outside the domain were denied; private sibling pathname sockets
were denied by their 0700 directory. Public pathname Unix sockets remained
connectable on ABI 6 and 8: filesystem read/write grants do not mediate those
connections. The approved design excludes shared local/network services from a
filesystem isolation claim. Do not expose unauthenticated file/debug services
there; private socket permissions and service authentication remain necessary.
Landlock also does not conceal all path metadata (`stat`, `chdir`, etc.).

## Tooling and build validation

Both container profiles passed Git init/commit/branch/merge/clone/gc; Python venv,
ordinary multiprocessing.Process, subprocesses and private tempfiles; Node and a
child process; npm local package install/import; C/C++ compile/link/run and make;
Cargo offline create/build/test/run; and pip bootstrap/local-wheel install/import.
Only Python multiprocessing.Pool failed in the completed tool matrix. Package
checks used local artifacts, with no registry quota dependency. This does not
assert compatibility with all packages or grant system-wide privileged installs.

Versions: Git 2.43.0, Python 3.12.3, Node 18.19.1, npm 9.2.0, GCC 13.3.0 and
Cargo 1.75.0 from the official Ubuntu runtime plus test packages. The native Pool
counterexample used Python 3.13. The prototype itself was built and linted with
**Rust 1.98.1**, not that distro Cargo.

Linux all-target Clippy (`-D warnings`), fmt and the three imported production
process tests at both default and `RUST_MIN_STACK=1572864` passed. The unavailable-
Landlock test also passed. No backend/frontend/full node regression suite was
needed for this unshipped standalone spike. The failed gate is retained, not
converted into an ignored check or a fallback.

## Reproduction

From the repository root on an arm64 Docker host:

```sh
docker buildx build --platform linux/arm64 --load \
  -f cli/tests/machine_context_spike/Dockerfile \
  -t nyxid-context-spike:local .
docker run --rm --security-opt no-new-privileges \
  --security-opt "seccomp=$PWD/cli/resources/machine-container/seccomp.json" \
  nyxid-context-spike:local --disposable
```

For mount denial, remove `mount`, `umount2` and `pivot_root` from every allow entry
in a copy of the seccomp profile, add an unconditional `SCMP_ACT_ERRNO` rule with
errno 1 for them, and use that copy in the same command. This approximates mount
denial; Docker Desktop itself did not run AppArmor.

For a disposable Linux VM, build the Dockerfile's `artifact` target with a local
output directory. Copy its binary to `/usr/local/bin/nyxid-context-spike` and copy
`check.py` into the VM. With Python and useradd installed:

```sh
sudo python3 check.py --disposable --native-profile --probes-only
```

Omit `--probes-only` to run the tool matrix after installing Git, Python venv/pip,
Node/npm, C/C++, make and Cargo. Each invocation needs a fresh disposable
filesystem. The command user must never choose a UID, root, allowlist or control
mode in a real implementation: these are supervisor-only experiment inputs.

All task containers, volumes, images and builder caches are cleaned up after the
spike. Keep the report and source, not an active sandbox with real credentials.

## Sources

- [Kernel Landlock userspace API](https://docs.kernel.org/userspace-api/landlock.html)
- [Linux 6.12 Landlock API](https://www.kernel.org/doc/html/v6.12/userspace-api/landlock.html)
- Production seccomp and identity implementation: `cli/src/node/machine/process.rs`.
- Official container seccomp: `cli/resources/machine-container/seccomp.json`.
