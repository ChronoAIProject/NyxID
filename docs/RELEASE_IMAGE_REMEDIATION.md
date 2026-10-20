# Image vulnerability remediation review (CI head 82e7605bb)

The final-image Trivy reports from CI 39412744887 remain the evidence source.
Raw HIGH/CRITICAL/UNKNOWN findings stay visible and blocking. No disposition or
severity change is introduced by this report.

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

There are no active image dispositions in this commit. The following inventory
is the **prior Debian-base baseline**, retained to identify every finding that
the minimal service images must remove. It contains 104 no-fixed-version
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

This document is a review inventory, not an active exception list. The image
gate continues to fail on these findings until fixes or approved exact
records with executable negative tests are implemented.
