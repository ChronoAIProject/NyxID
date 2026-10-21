#!/usr/bin/env python3
import copy
import importlib.util
import json
import os
from pathlib import Path
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[1]
spec = importlib.util.spec_from_file_location('signing_tools', ROOT / 'scripts/check-signing-tools.py')
policy = importlib.util.module_from_spec(spec); spec.loader.exec_module(policy)


class LockedProducer(unittest.TestCase):
    def setUp(self):
        self.source = Path(os.environ.get('SIGNING_TOOLS_ROOT', ROOT / 'security/signing-tools'))
        temp = tempfile.TemporaryDirectory(); self.addCleanup(temp.cleanup); self.root = Path(temp.name)
        self.original = {n: json.loads((self.source / n).read_text()) for n in ('package.json', 'package-lock.json')}
        self.write(self.original)

    def write(self, values):
        for name, value in values.items(): (self.root / name).write_text(json.dumps(value))

    def test_reviewed_lock(self): self.assertGreater(policy.check(self.root), 1)

    def test_unlocked_versions_missing_security_override_hooks_and_approval_fail(self):
        for mutation in ('range', 'override', 'script', 'allow'):
            values = copy.deepcopy(self.original); package = values['package.json']
            if mutation == 'range': package['dependencies']['@sigstore/sign'] = '^3.0.0'
            elif mutation == 'override': del package['overrides']['@sigstore/core']
            elif mutation == 'script': package['scripts'] = {'postinstall': 'false'}
            else: package['allowScripts'] = {'*': True}
            self.write(values)
            with self.subTest(mutation=mutation), self.assertRaises(ValueError): policy.check(self.root)

    def test_mutable_or_unverified_archives_and_install_hooks_fail(self):
        for mutation in ('no-integrity', 'sha1', 'host', 'http', 'hook', 'link', 'core'):
            values = copy.deepcopy(self.original); packages = values['package-lock.json']['packages']
            entry = packages['node_modules/@sigstore/core']
            if mutation == 'no-integrity': del entry['integrity']
            elif mutation == 'sha1': entry['integrity'] = 'sha1-' + 'A' * 28
            elif mutation == 'host': entry['resolved'] = 'https://untrusted.invalid/pkg.tgz'
            elif mutation == 'http': entry['resolved'] = entry['resolved'].replace('https:', 'http:')
            elif mutation == 'hook': entry['hasInstallScript'] = True
            elif mutation == 'link': entry['link'] = True
            else: entry['version'] = '3.2.0'
            self.write(values)
            with self.subTest(mutation=mutation), self.assertRaises(ValueError): policy.check(self.root)


if __name__ == '__main__': unittest.main()
