#!/usr/bin/env python3
"""Fail closed on probe inputs, daemon leftovers and registration/reconnect failures."""
import importlib.util
import copy
from pathlib import Path
import ssl
import subprocess
import tempfile
import unittest
from unittest.mock import patch

spec = importlib.util.spec_from_file_location('machine_docker_cli', Path(__file__).with_name('check-machine-docker-cli.py'))
checks = importlib.util.module_from_spec(spec)
spec.loader.exec_module(checks)


class ProbeBoundary(unittest.TestCase):
    def setUp(self):
        self.args = ['run', '--security-opt', 'seccomp=/reviewed.json', '--init', '--shm-size', '512m',
                     '--rm', '--entrypoint', '/usr/local/bin/nyxid', checks.OFFICIAL,
                     'node', 'setup', '--container', '--probe-separation']

    def test_actual_official_image_probe_has_no_identity_inputs(self):
        checks.require_probe(self.args)

    def test_token_or_environment_injection_rejects(self):
        for value in ('-e', '-eNYXID_NODE_TOKEN=x', '--env', '--env=NYXID_NODE_TOKEN=x', '--env-file', '--env-file=/tmp/env'):
            with self.subTest(value=value), self.assertRaises(ValueError): checks.require_probe(self.args + [value])

    def test_state_mount_or_anonymous_container_leak_rejects(self):
        for value in ('-v', '-vstate:/state', '--volume', '--volume=state:/state', '--volumes-from=identity', '--mount', '--mount=type=bind'):
            with self.subTest(value=value), self.assertRaises(ValueError): checks.require_probe(self.args + [value])
        self.args.remove('--rm')
        with self.assertRaises(ValueError): checks.require_probe(self.args)

    def test_other_image_or_missing_real_probe_rejects(self):
        for value in (checks.OFFICIAL, '--probe-separation'):
            with self.subTest(value=value), self.assertRaises(ValueError): checks.require_probe([arg for arg in self.args if arg != value])

    def test_new_probe_container_or_volume_cannot_be_ignored(self):
        checks.require_unchanged(({'existing'}, {'existing-volume'}), ({'existing'}, {'existing-volume'}))
        for after in (({'existing', 'probe'}, {'existing-volume'}), ({'existing'}, {'existing-volume', 'anonymous'})):
            with self.subTest(after=after), self.assertRaises(ValueError):
                checks.require_unchanged(({'existing'}, {'existing-volume'}), after)


class FixtureAcceptance(unittest.TestCase):
    def state(self, **changes):
        return dict(errors=0, registration_attempts=1, registrations=1, statuses=1, **changes)

    def test_private_ipc_fixture_has_verified_tls_and_stops_cleanly(self):
        with tempfile.TemporaryDirectory() as temporary:
            with checks.fixture(Path(temporary), '127.0.0.1') as (info, certificate, state):
                self.assertTrue(certificate.is_file())
                self.assertTrue(info['registration_token'].startswith('nyx_nreg_'))
                current = state()
                self.assertEqual(current['registration_attempts'], 0)
                self.assertEqual(current['connections'], 0)
                self.assertEqual(current['requests'], 0)
                self.assertNotIn(info['registration_token'], str(current))

    def test_distinct_server_leaf_chain_constraints_and_gateway_san(self):
        with tempfile.TemporaryDirectory() as temporary:
            directory = Path(temporary)
            ca, leaf, key = checks.generate_tls(directory, '172.17.0.1')
            self.assertNotEqual(ca.read_bytes(), leaf.read_bytes())
            ca_text, leaf_text = [subprocess.check_output(['openssl', 'x509', '-in', str(path), '-noout', '-text'], text=True)
                                  for path in (ca, leaf)]
            self.assertIn('CA:TRUE', ca_text)
            self.assertIn('CA:FALSE', leaf_text)
            self.assertIn('TLS Web Server Authentication', leaf_text)
            self.assertIn('IP Address:127.0.0.1', leaf_text)
            self.assertIn('IP Address:172.17.0.1', leaf_text)
            self.assertEqual(key.stat().st_mode & 0o777, 0o600)
            self.assertEqual((directory / 'ca.key').stat().st_mode & 0o777, 0o600)
            subprocess.run(['openssl', 'verify', '-CAfile', str(ca), '-purpose', 'sslserver', str(leaf)],
                           check=True, capture_output=True)
            # A server-only leaf cannot also serve as a client certificate.
            self.assertNotEqual(subprocess.run(['openssl', 'verify', '-CAfile', str(ca), '-purpose', 'sslclient', str(leaf)],
                                               capture_output=True).returncode, 0)

    def test_served_leaf_requires_our_ca_and_matching_ip(self):
        with tempfile.TemporaryDirectory() as temporary:
            directory = Path(temporary)
            with checks.fixture(directory, '172.17.0.1') as (info, ca, state):
                state()  # Real TLS to loopback with CA-only trust succeeds.
                import socket
                for context, hostname in ((ssl.create_default_context(), '127.0.0.1'),
                                          (ssl.create_default_context(cafile=str(ca)), 'wrong.fixture.invalid')):
                    with self.subTest(hostname=hostname), socket.create_connection(('127.0.0.1', info['port'])) as connection:
                        with self.assertRaises(ssl.SSLCertVerificationError):
                            context.wrap_socket(connection, server_hostname=hostname)

    def test_gateway_cannot_inject_certificate_extensions(self):
        with tempfile.TemporaryDirectory() as temporary, self.assertRaises(ValueError):
            checks.generate_tls(Path(temporary), '127.0.0.1\nbasicConstraints=CA:TRUE')

    def test_expected_real_fixture_state_is_required(self):
        self.assertEqual(checks.wait_for(lambda: self.state(), lambda item: item['statuses'] == 1)['registrations'], 1)

    def test_protocol_error_or_duplicate_registration_fails_even_if_predicate_succeeds(self):
        for key in ('errors', 'registration_attempts'):
            state = self.state()
            state[key] = 2
            with self.subTest(key=key), self.assertRaises(ValueError): checks.wait_for(lambda: state, lambda _: True)

    def test_missing_handshake_never_reports_success(self):
        with self.assertRaisesRegex(ValueError, 'did not reach'):
            checks.wait_for(lambda: self.state(), lambda _: False, timeout=-1)

    def test_failure_diagnostics_redact_synthetic_token_and_signing_material(self):
        for token in ('nyx_' + 'nreg_' + 'a' * 64, 'nyx_' + 'nauth_' + 'b' * 64, 'c' * 64):
            self.assertNotIn(token, checks.sanitized('failure: ' + token))
        self.assertIn('fixed failure reason', checks.sanitized('fixed failure reason'))

    def test_docker_observer_only_records_then_execs_real_client(self):
        import json, os, subprocess, sys
        with tempfile.TemporaryDirectory() as temporary:
            directory = Path(temporary)
            client = directory / 'actual-docker'
            record = directory / 'real-client-args'
            client.write_text('#!' + sys.executable + '\nimport json,sys\nfrom pathlib import Path\n' +
                             'Path(' + repr(str(record)) + ').write_text(json.dumps(sys.argv[1:]))\nraise SystemExit(17)\n')
            client.chmod(0o700)
            with patch.object(checks.shutil, 'which', return_value=str(client)):
                observer, trace = checks.observe_docker(directory)
            args = ['run', '--rm', checks.OFFICIAL, 'node', 'setup', '--probe-separation']
            result = subprocess.run([str(observer / 'docker'), *args])
            self.assertEqual(result.returncode, 17)
            self.assertEqual(json.loads(record.read_text()), args)
            self.assertEqual(json.loads(trace.read_text()), args)


class RetainedContainer(unittest.TestCase):
    def setUp(self):
        self.name = 'nyxid-node-machine-fixture'
        self.image = 'sha256:' + 'b' * 64
        self.info = dict(Id='a' * 64, Image=self.image, State=dict(Running=True),
                         Config=dict(Env=['NYXID_MACHINE_MANAGED=0', 'NYXID_MACHINE_SEPARATION=shared-legacy']),
                         Mounts=[dict(Type='volume', Name=self.name + '-state', Destination='/var/lib/nyxid-machine', RW=True),
                                 dict(Type='volume', Name=self.name + '-workspace', Destination='/workspace', RW=True)])

    def test_stop_retains_identity_volumes_and_choice(self):
        self.assertEqual(checks.require_machine_container(self.info, self.name, self.image), 'a' * 64)
        self.info['State']['Running'] = False
        checks.require_machine_container(self.info, self.name, self.image, 'a' * 64, running=False)
        self.info['State']['Running'] = True
        checks.require_machine_container(self.info, self.name, self.image, 'a' * 64)

    def test_replaced_container_changed_choice_wrong_volumes_or_running_state_fail(self):
        for mutation in ('id', 'choice', 'managed', 'state-volume', 'workspace-volume', 'bind', 'readonly', 'duplicate', 'running'):
            info = copy.deepcopy(self.info)
            if mutation == 'id': info['Id'] = 'c' * 64
            elif mutation == 'choice': info['Config']['Env'][1] = 'NYXID_MACHINE_SEPARATION=automatic'
            elif mutation == 'managed': info['Config']['Env'][0] = 'NYXID_MACHINE_MANAGED=1'
            elif mutation == 'state-volume': info['Mounts'][0]['Name'] = 'other-state'
            elif mutation == 'workspace-volume': info['Mounts'][1]['Name'] = 'other-workspace'
            elif mutation == 'bind': info['Mounts'][0]['Type'] = 'bind'
            elif mutation == 'readonly': info['Mounts'][0]['RW'] = False
            elif mutation == 'duplicate': info['Mounts'].append(dict(info['Mounts'][0]))
            else: info['State']['Running'] = False
            with self.subTest(mutation=mutation), self.assertRaises(ValueError):
                checks.require_machine_container(info, self.name, self.image, 'a' * 64)

    def test_refused_profile_cannot_leave_only_hidden_metadata_or_dangling_link(self):
        with tempfile.TemporaryDirectory() as temporary:
            profile = Path(temporary) / 'refused-profile'
            checks.require_profile_absent(profile)
            profile.mkdir()
            for name in ('.keyfile', '.config.toml.tmp', '.nyxid-storage-file',
                         '.secret-intent-fixture.json', '.secret-attempt-fixture.lock'):
                marker = profile / name
                marker.touch()
                with self.subTest(name=name), self.assertRaises(ValueError):
                    checks.require_profile_absent(profile)
                marker.unlink()
            profile.rmdir()
            profile.symlink_to(Path(temporary) / 'absent-target')
            with self.assertRaises(ValueError): checks.require_profile_absent(profile)


if __name__ == '__main__':
    unittest.main()
