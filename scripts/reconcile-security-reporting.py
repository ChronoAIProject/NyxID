#!/usr/bin/env python3
"""Restore/check NyxID's private-reporting link after managed template output.

This is a local tool, not an fkst-hosted hook. Upstream must explicitly invoke
an equivalent merge before committing its generated chooser; see the tracker.
"""
import argparse
from pathlib import Path
import sys

CONTACT = """  - name: Report a security vulnerability privately
    url: https://github.com/ChronoAIProject/NyxID/security/advisories/new
    about: Do not disclose vulnerabilities or secrets in public issues. See SECURITY.md.
"""

# Deterministic upstream-compatible contact. Generic generators must route via
# the repository policy; private advisory reporting may be disabled elsewhere.
POLICY_CONTACT = """  - name: Security policy and vulnerability reporting
    url: https://github.com/ChronoAIProject/NyxID/security/policy
    about: Read the repository security policy before reporting vulnerabilities. Do not disclose vulnerabilities or secrets in public issues.
"""


def reconcile(content: str) -> str:
    if CONTACT in content or POLICY_CONTACT in content:
        return content
    if content.count("\ncontact_links:\n") != 1:
        raise ValueError("expected one top-level contact_links list; review the generated chooser")
    # Fixed repository-owned text only. Preserve the generator marker, other
    # links and blank_issues_enabled verbatim. Fail on unexpected config shape.
    return content.replace("\ncontact_links:\n", "\ncontact_links:\n" + CONTACT, 1)


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--check", action="store_true")
    parser.add_argument("path", nargs="?", type=Path, default=Path(".github/ISSUE_TEMPLATE/config.yml"))
    args = parser.parse_args()
    content = args.path.read_text()
    result = reconcile(content)
    if args.check:
        if result != content:
            print("Private security reporting contact missing from managed issue chooser", file=sys.stderr)
            return 1
    else:
        args.path.write_text(result)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
