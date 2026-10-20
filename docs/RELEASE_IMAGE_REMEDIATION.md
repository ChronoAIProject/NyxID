# Image vulnerability remediation review (CI head 82e7605bb)

The final-image Trivy reports from CI 39412744887 remain the evidence source.
Raw HIGH/CRITICAL/UNKNOWN findings stay visible. Only the exact reviewed
header-package findings below can pass the executable eligibility gate; every
other finding remains blocking and no severity changes.

Machine workstation images retain `build-essential` and its header closure for
C/C++ compile/link/run, make, npm native installs and pip local wheels. The
rejected toolchain removal has been reversed. Hosted desktop keeps the verified
upstream gh 2.102.0 release instead of the vulnerable Ubuntu 2.45.0 package.

The MCP demo now builds a static musl executable in the pinned Alpine Rust
builder and ships `FROM scratch`, with no OS packages or shell. The interim
liblzma5 upgrade is superseded. The updater and MCP demo are the only explicit
`scratch_components` in the scanner and SBOM policy. Their embedded Rust
inventories, architecture/static checks, final scans and subject-bound SBOMs
remain mandatory. New native image builds/scans are needed to measure the
resulting finding counts; the prior demo report had 46 blocking findings.

## Node runtime package and linkage evidence

Readelf inspection of both actual Linux CLI archives from CI 39412744887
(`pr-cli-archive-x86_64-unknown-linux-gnu` and
`pr-cli-archive-aarch64-unknown-linux-gnu`) found `libdbus-1.so.3` in DT_NEEDED.
The amd64 archive additionally needs libgcc_s.so.1, libm.so.6, libc.so.6 and
ld-linux-x86-64.so.2. The arm64 archive additionally needs libgcc_s.so.1,
libpthread.so.0, libm.so.6, libdl.so.2 and libc.so.6. These are the actual release
archives, not binaries extracted from the image: CI did not retain that image.
Running the arm64 archive in the exact pinned bookworm-slim base fails before
main with a missing libdbus-1.so.3 loader error. The Dockerfile comment claiming
that the encrypted file backend removes the loader dependency is incorrect;
unconditional keyring Secret Service features introduce it even when unused.
Reconstructing the current final Dockerfile stage, including its real snapshot
CA installation, around that CI arm64 executable reproduces exit 127 from
`docker run --rm --entrypoint /usr/local/bin/nyxid ... --version`.
`dpkg-query` confirms ca-certificates is installed and libdbus-1-3 is absent.
This reconstructs the final stage; it is not the historical image digest that
the runner did not retain.

The new exporter retains exactly the following runtime packages, their dpkg
status/control records and actual file lists. A native arm64 disposable replay
used the October 20 signed snapshots and produced these versions. Only the
aggregate CA bundle is copied from that snapshot's ca-certificates package;
its openssl, debconf and shell dependencies remain in the package builder.
The exported filesystem's Trivy scan detected exactly five OS packages and
zero HIGH/CRITICAL/UNKNOWN findings with the existing local vulnerability DB.
This is rootfs evidence; the completed node image still requires native PR
execution, Rust inventory, current vulnerability scan and subject-bound SBOM.

| Retained package | Native arm64 replay version | Runtime role |
| --- | --- | --- |
| base-files | 12.4+deb12u15 | Distribution identity, base directory and configuration files |
| libc6 | 2.36-9+deb12u14 | GNU loader, libc, libm, pthread/dl compatibility and resolver support |
| libgcc-s1 | 12.2.0-14+deb12u1 | Compiler runtime and unwinding support |
| netbase | 6.4 | Network service/protocol databases |
| tzdata | 2026c-0+deb12u1 | Timezone data |
| CA bundle only, ca-certificates | 20250419~deb12u1 | Trust bundle SHA-256: 714d457d580922dbf1d0be8bd35ba236a842b50b0072ae791582a19adef772a5 |

The shell entrypoint currently invokes only nyxid, plus shell builtins (`set`,
`[`, `echo`, `exit`, `exec`). Node SSH execution uses russh rather than an
external ssh client. The reduced node image needs no shell once registration
and foreground-start orchestration move to the coordinated CLI subcommand;
no ssh client, util-linux tools/libraries, perl, ncurses, gzip, libacl,
libsystemd/libudev, OpenSSL or zlib is in the five-package export. All packages
behind the prior 60 Debian findings are excluded. The CLI container feature
must also remove the DBus link; adding the old CLI to this rootfs would fail.
This service image is distinct from the machine workstation images, whose
owner shell/build/browser workloads retain their documented tools.

The signed October 20 noble-security native package indexes for both amd64 and
arm64 still offer linux-libc-dev/source linux `6.8.0-147.147`. Their sizes and
SHA-256 hashes were checked against the catalog-bound signed InRelease.

| Component | Last hosted inventory (amd64 / arm64 OS packages) | Blocking findings per architecture | New native evidence |
| --- | --- | --- | --- |
| node-agent | 91 / 91 | 60 | Five-package exported rootfs: 0; completed image awaits the CLI DBus/entrypoint change |
| mcp-demo | 78 / 78 | 46 | Static scratch fixture: no OS packages, 11 embedded Rust packages, 0 blocking findings |
| nyxid-node-machine | 740 / 737 | 309, all linux-libc-dev | Toolchain retained; 309 exact header records per architecture |
| machine-node-only | 553 / 552 | 309, all linux-libc-dev | Toolchain retained; 309 exact header records per architecture |
| machine-hosted-desktop / machine-hosted | No retained complete report | New scan required | 309 exact header records per component/architecture; final scan must match; gh 2.102.0 retained |
| nyxid-machine-updater | 0 / 0 | 0 | Existing static scratch image unchanged |

The native scans use the existing local vulnerability database. Only new PR
jobs can establish current counts for both rebuilt final architectures. The
workstation package counts above describe the last hosted image, not a newly
built final inventory.

The mandatory final identity checker also executes the actual immutable image.
Every CLI image must run `nyxid --version`, `nyxid --help`, and its configured
entrypoint's sole `--help` path. Backend/frontend executables, the MCP stdio
ping, and the updater's side-effect-free invalid-command path are exercised
as well. A loader error, timeout, wrong response or failed exit blocks PR
acceptance, publication and the operator wrapper before it applies a tag.
Probes run without networking or registration input, with a read-only root,
dropped capabilities and bounded resources; cleanup removes anonymous volumes.
The workstation Dockerfiles use a fixed help/version dispatcher ahead of their
existing entrypoints so these probes perform no registration or startup writes.
Ordinary invocation preserves the original arguments and environment.

## Approved residual-finding policy

A final-image finding is disposition-eligible only when all hold: (a) the
package is in the component's declared minimal runtime set; (b) the pinned
suite's security archive at the signed snapshot used by the build offers no
fixed version for the source package (an upstream source fix does not make the
finding ineligible; NyxID does not self-build Debian packages); (c) a reachability
rationale names the affected executable/API and an executable negative test
proves it absent or unreachable in the final filesystem; (d) owner, 30-day
expiry, bound to CVE + source package + binary package + version + component +
architecture. A fixed version in a later snapshot catalog, any new
CVE/package/version/content, or expiry fails the gate; no renewal while a fix
exists.

A CRITICAL always blocks when the vulnerable code is in a library the shipped
binary links (DT_NEEDED), in an executable reachable from the entrypoint, or in
code that processes network- or user-supplied input.

The scanner implements only the closed `linux-uapi-package-has-no-code-v1`
negative test. An exact record requires the actual workstation dependency chain
`build-essential -> libc6-dev -> linux-libc-dev`, source/binary versions,
component, architecture, full reviewed advisory content, a named affected API,
owner, issue date and an expiry no more than 30 days later. The entire reviewed
snapshot catalog and final package file-list/content digest are bound. The
security archive's native Packages.xz is verified against its SHA-pinned signed
InRelease, and its candidate/source version must match the reviewed version.
A reported FixedVersion always rejects a record, including a renewal. Missing
or resolved advisory identities require removing stale records.

The host inspects a stopped immutable image's real dpkg file list and exported
filesystem; no image-provided program is trusted to assert absence of code.
Only regular, non-executable, non-ELF header files under `/usr/include` pass,
with exactly two additional regular files allowed:
`/usr/share/doc/linux-libc-dev/changelog.Debian.gz` and
`/usr/share/doc/linux-libc-dev/copyright`, plus their parent directories.
These documents must also be non-ELF and non-executable, and their bytes are
bound by the package-content digest. Any other document, non-header path,
symlink, executable or ELF object fails. Tests cover each forbidden case and
changed document bytes. All active records are validated for expiry even when
unrelated images are being scanned.
Publication replay checks the same policy and reruns the filesystem proof;
missing/changed evidence cannot become an unsigned summary-based pass.

The policy records 309 exact CVE identities per component and architecture for
`nyxid-node-machine`, `machine-node-only`, `machine-hosted-desktop` and
`machine-hosted`: **2,472 records**, all source `linux` / binary `linux-libc-dev`
`6.8.0-147.147`. Each group contains 298 HIGH and 11 CRITICAL findings. Records
name the affected kernel operation and reviewer, were issued 2026-10-20, and
expire exclusively on 2026-11-19 (30 days). The retained node-machine and
node-only reports have identical reviewed header-advisory content on both
architectures. Hosted records cover that same exact package/advisory set;
there is no retained complete hosted report, so they cannot establish that its
next final scan has no additional findings.

Both native security indexes still offer only `6.8.0-147.147`. Authenticated
package archives were unpacked with dpkg in disposable scratch proof images;
the actual collector inspected the dpkg file lists and every payload file.
The amd64 payload has 988 headers plus the two documents; arm64 has 960 headers
plus the two documents. Their bound content digests are respectively
`064b54e8c4e2881d001e0ae80600f1c7d303732f58228e9eb41c4c347ad7b7e2`
and `3011922dc65fff693e9b67bfacdc14d3502931f5cb83ada6fd9868413ecdeac5`.
This proves the exact package payloads; each rebuilt final workstation image
must independently pass the same filesystem, source, candidate and advisory
checks in PR CI or the private hosted operator path.

The following inventory is the **prior Debian-base baseline**, retained to
identify every finding that the minimal service images must remove. It contains 104 no-fixed-version
component/package/version/CVE combinations (60 node-agent, 44 MCP demo), repeated
across both architectures, plus the demo's two fixed liblzma5 findings.

## Prior Debian-base findings

| Advisory | Component: exact reported packages and versions | Status |
| --- | --- | --- |
| CVE-2023-45853 | node-agent: `1:1.2.13.dfsg-1` (zlib1g) | will_not_fix |
| CVE-2025-69720 | mcp-demo: `6.5+20250216-2` (libtinfo6, ncurses-base, ncurses-bin)<br>node-agent: `6.4-4` (libtinfo6, ncurses-base, ncurses-bin) | affected |
| CVE-2026-16742 | mcp-demo: `257.13-1~deb13u1` (libsystemd0, libudev1)<br>node-agent: `252.39-1~deb12u2` (libsystemd0, libudev1) | affected, fix_deferred |
| CVE-2026-41992 | node-agent: `1.12-1` (gzip) | fix_deferred |
| CVE-2026-53613 | node-agent: `1:2.38.1-5+deb12u3` (bsdutils); `2.38.1-5+deb12u3` (libblkid1, libmount1, libsmartcols1, libuuid1, mount, util-linux, util-linux-extra) | affected |
| CVE-2026-53615 | node-agent: `1:2.38.1-5+deb12u3` (bsdutils); `2.38.1-5+deb12u3` (libblkid1, libmount1, libsmartcols1, libuuid1, mount, util-linux, util-linux-extra) | affected |
| CVE-2026-54369 | mcp-demo: `2.3.2-2+b1` (libacl1)<br>node-agent: `2.3.1-3` (libacl1) | affected, fix_deferred |
| CVE-2026-76642 | mcp-demo: `1:2.41.5-0+deb13u1` (bsdutils); `1:4.16.0-2+really2.41.5-0+deb13u1` (login); `2.41.5-0+deb13u1` (libblkid1, liblastlog2-2, libmount1, libsmartcols1, libuuid1, mount, util-linux)<br>node-agent: `1:2.38.1-5+deb12u3` (bsdutils); `2.38.1-5+deb12u3` (libblkid1, libmount1, libsmartcols1, libuuid1, mount, util-linux, util-linux-extra) | affected |
| CVE-2026-78408 | mcp-demo: `1:2.41.5-0+deb13u1` (bsdutils); `1:4.16.0-2+really2.41.5-0+deb13u1` (login); `2.41.5-0+deb13u1` (libblkid1, liblastlog2-2, libmount1, libsmartcols1, libuuid1, mount, util-linux)<br>node-agent: `1:2.38.1-5+deb12u3` (bsdutils); `2.38.1-5+deb12u3` (libblkid1, libmount1, libsmartcols1, libuuid1, mount, util-linux, util-linux-extra) | affected |
| CVE-2026-78409 | mcp-demo: `1:2.41.5-0+deb13u1` (bsdutils); `1:4.16.0-2+really2.41.5-0+deb13u1` (login); `2.41.5-0+deb13u1` (libblkid1, liblastlog2-2, libmount1, libsmartcols1, libuuid1, mount, util-linux)<br>node-agent: `1:2.38.1-5+deb12u3` (bsdutils); `2.38.1-5+deb12u3` (libblkid1, libmount1, libsmartcols1, libuuid1, mount, util-linux, util-linux-extra) | affected |
| CVE-2026-78410 | mcp-demo: `1:2.41.5-0+deb13u1` (bsdutils); `1:4.16.0-2+really2.41.5-0+deb13u1` (login); `2.41.5-0+deb13u1` (libblkid1, liblastlog2-2, libmount1, libsmartcols1, libuuid1, mount, util-linux)<br>node-agent: `1:2.38.1-5+deb12u3` (bsdutils); `2.38.1-5+deb12u3` (libblkid1, libmount1, libsmartcols1, libuuid1, mount, util-linux, util-linux-extra) | affected |
| CVE-2026-82560 | mcp-demo: `5.40.1-6+deb13u1` (perl-base)<br>node-agent: `5.36.0-7+deb12u4` (perl-base) | affected |
| CVE-2026-84782 | node-agent: `3.0.22-1~deb12u1` (libssl3, openssl) | affected |
| CVE-2026-9538 | mcp-demo: `5.40.1-6+deb13u1` (perl-base)<br>node-agent: `5.36.0-7+deb12u4` (perl-base) | fix_deferred |

## Reachability evidence required for any remaining package

- CVE-2023-45853 concerns the separately packaged MiniZip code, not the ordinary
  zlib compression API. Verify the final filesystem has no affected MiniZip or
  pyminizip copy; retain Debian's source-package status and package-version bind.
- CVE-2026-16742 requires systemd-homed. Reports identify libsystemd0/libudev1
  through the source package; verify the affected homed executable and service
  are absent from the actual image.
- CVE-2026-9538 and CVE-2026-82560 concern Archive::Tar and Pod::Text. Verify the
  vulnerable modules are absent rather than assuming perl-base omits them.
- CVE-2025-69720 concerns the infocmp executable. libtinfo6/ncurses-base source
  attribution alone does not establish reachability; ncurses-bin can contain
  the actual affected executable and needs separate treatment.
- The mount/nsenter and libblkid advisories concern privileged local mount,
  cgroup or partition-probe paths. Do not describe these as unreachable merely
  because NyxID is a network client: container execution settings and the
  operator's enabled node capabilities matter.
- libacl, gzip and OpenSSL require per-use review. NyxID's Rust TLS and archive
  implementations do not alone prove that every retained OS tool avoids the
  affected API (including OpenSSL DTLS). A monthly risk decision needs an
  explicit, verified scope and rationale; a blanket source-package exception
  would lose that distinction.

These Debian baseline findings have no dispositions. The image gate continues
to fail on them until the minimal service images remove their packages. Active
header-only records live in `security/image-dispositions.json`, with all of the
exact eligibility checks above.

## Pure Rust keychain and shell-free node runtime

The CLI uses keyring 3.6.3 with `async-secret-service`, `async-io` and
`crypto-rust`, alongside the native Apple/Windows backends. The async-io choice
avoids keyring's documented Tokio main-thread deadlock. Secret Service 4.0.0
cannot compile without a crypto implementation, so the reviewed feature set
changes its D-Bus session negotiation from **Plain to Dh** using RustCrypto,
without OpenSSL or libdbus. Native keychain APIs and credential names stay the
same. Cargo may retain weak optional dbus packages in its lock resolution;
`cargo tree --target <linux-target> -i libdbus-sys` must have no active parent,
and the actual Linux archives and images must have no libdbus DT_NEEDED entry.
The dependency audit and exact lock-subgraph guard remain mandatory.

`nyxid node docker-entrypoint` now performs the old container registration and
start sequence in Rust. Existing config starts directly; missing config uses
`NYXID_NODE_TOKEN` and `NYXID_NODE_URL` to register with file custody, and missing
config plus absent token refuses with exit 1. `NYXID_NODE_CONFIG_DIR` defaults to
`/app/config`; start arguments are parsed before consuming registration input.
The node image contains exactly the five declared snapshot runtime packages
and the same snapshot's CA bundle, with real dpkg inventory retained for scans.
It contains no shell, SSH client, util-linux, Perl, ncurses or DBus library.

Required CLI Test runs the actual CLI test executable in a disposable, unlocked
gnome-keyring/D-Bus session, with preflight/set/get/delete inside Tokio and a
90-second deadlock bound. A native arm64 Lima replay of the unchanged production
KeychainBackend also passed these operations in Tokio; that small fixture does
not replace the hosted actual-CLI check. Both Linux archive checks parse the
actual ELF dependencies, and every final CLI image executes version/help and
its real entrypoint. Node additionally proves no-config startup exits 1.
