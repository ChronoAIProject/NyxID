#!/usr/bin/env python3
"""Check NyxID's managed issue chooser for a private-reporting route (read-only)."""
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


def has_security_contact(content: str) -> bool:
    return CONTACT in content or POLICY_CONTACT in content


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("path", nargs="?", type=Path, default=Path(".github/ISSUE_TEMPLATE/config.yml"))
    args = parser.parse_args()
    content = args.path.read_text()
    if not has_security_contact(content):
        print("Private security reporting contact missing from managed issue chooser", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
