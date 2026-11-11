#!/usr/bin/env python3
"""Time the shipped capability probes in an expendable Linux guest/image.

No registration is performed. Container mode needs the real full machine image,
its normal seccomp profile, --init and shm setting, with no token environment or
identity/workspace volumes. Native mode reads an explicitly named fixture config.
"""
import argparse
import math
import os
import re
import statistics
import subprocess
import json

parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument('--disposable', action='store_true', required=True)
parser.add_argument('--binary', default='/usr/local/bin/nyxid')
parser.add_argument('--samples', type=int, default=100)
mode = parser.add_mutually_exclusive_group(required=True)
mode.add_argument('--container', action='store_true')
mode.add_argument('--config')
args = parser.parse_args()
assert os.uname().sysname == 'Linux' and os.geteuid() == 0
assert 1 <= args.samples <= 1000
mechanisms = ('CommandSeccomp', 'Landlock', 'BrowserPolicy', 'SecureBrowser', 'DeveloperBrowser')
measures = {name: [] for name in mechanisms}
command = [args.binary, 'node', 'setup', '--probe-separation']
command += ['--container'] if args.container else ['--config', args.config]
environment = {key: value for key, value in os.environ.items() if not key.startswith('NYXID_')}
for sample in range(args.samples):
    result = subprocess.run(command, env=environment, capture_output=True, text=True, check=False)
    # Never print private browser/OS errors or arbitrary subprocess output.
    # These production labels identify the failed observation before assertions.
    for line in result.stderr.splitlines():
        if line == 'separation_probe_unavailable reason=separated_browser_sandbox_unobservable_in_container' or re.fullmatch(r'browser_probe_failure reason=[a-z_]+ errno=(?:[0-9]+|unknown)', line) or re.fullmatch(
            r'separation_probe mechanism=\w+ elapsed_ms=\d+ outcome=failed', line
        ):
            print(f'probe_failure sample={sample + 1} {line}', flush=True)
    assert result.returncode == 0, f'capability probe command failed (exit {result.returncode})'
    evidence = json.loads(result.stdout)
    assert evidence.get('version') == 4 and evidence.get('os') == 'linux'
    for field in ('separate_users', 'managed_browser_configured', 'command_seccomp', 'landlock_enforced', 'managed_browser_sandbox', 'managed_browser_policy', 'credential_custody'):
        assert evidence.get(field) is True, f'probe did not confirm {field} in sample {sample + 1}'
    assert evidence.get('landlock_abi', 0) >= 6 and not evidence.get('probe_timed_out')
    seen = set()
    for line in result.stderr.splitlines():
        match = re.fullmatch(r'separation_probe mechanism=(\w+) elapsed_ms=(\d+) outcome=(confirmed|failed)', line)
        if not match:
            continue
        name, duration, outcome = match.groups()
        assert name in measures and name not in seen and outcome == 'confirmed'
        seen.add(name)
        measures[name].append(int(duration))
        print(f'probe_timing sample={sample + 1} mechanism={name} elapsed_ms={duration}', flush=True)
    assert seen == set(mechanisms), 'missing or duplicate mechanism timings'
for name, values in measures.items():
    ordered = sorted(values)
    p99 = ordered[math.ceil(len(ordered) * .99) - 1]
    print(f'probe_summary mechanism={name} samples={len(values)} median_ms={statistics.median(values):g} observed_p99_ms={p99} max_ms={max(values)}')
    if name in ('BrowserPolicy', 'SecureBrowser', 'DeveloperBrowser'):
        candidate = max(5000, math.ceil(p99 * 2 / 1000) * 1000)
        print(f'probe_bound_review mechanism={name} candidate_ms={candidate} rule=two_times_observed_p99_with_existing_5000ms_floor')
print(f'PASS production capability timing: {args.samples} reports, {args.samples * len(mechanisms)} successful mechanism observations')
