"""Exact, expiring dispositions for scanner findings; never a detector allowlist."""
from datetime import date, datetime, timezone, timedelta
import hashlib
import json
from pathlib import PurePosixPath
import re


def digest(value):
    return hashlib.sha256(value).hexdigest()


def normalized_path(value):
    if not isinstance(value, str) or not value or any(char in value for char in '\\\x00*?[]'):
        raise ValueError('invalid scanner path')
    path = PurePosixPath(value)
    if path.is_absolute() or '..' in path.parts or not path.parts:
        raise ValueError('scanner path must be relative without traversal')
    return path.as_posix()


IDENTITY = ('rule', 'path', 'match_sha256', 'value_sha256', 'content_sha256')
KINDS = {'documentation-placeholder', 'synthetic-document-example', 'synthetic-test-fixture',
         'non-secret-identifier', 'public-client-config', 'inert-historical-credential'}

INERT_DEVICE_REFRESH_EVIDENCE = {
    'credential_class': 'device-code-refresh-token',
    'issuance': 'backend/src/services/device_code_service/approve.rs:118,165 writes device_codes.refresh_token_hash only',
    'no_redemption': 'PM code review 2026-10-20: no handler/service redemption path looks up device_codes.refresh_token_hash; /auth/refresh requires a JWT jti',
    'retention': 'DEVICE_CODE_DELIVERY_EXPIRES_IN_SECS=86400 after approval; backend/src/db.rs TTL index on device_codes.expires_at',
    'unshipped_provenance': 'b73382431229:docs/device-code-binding.md Migration Notes states this feature has not shipped with production data',
}
INERT_DEVICE_REFRESH_HASH = 'a9d3c990476cf66f93e1851f7d429699e7b1597f03cf778fc15b721b86288b66'  # One PM-reviewed historical identity.



def load_exceptions(path, today=None):
    today = today or datetime.now(timezone.utc).date()
    data = json.loads(path.read_text())
    if set(data) != {'schema_version', 'exceptions'} or data['schema_version'] != 1:
        raise ValueError('invalid secret disposition schema')
    entries = data['exceptions']
    if not isinstance(entries, list):
        raise ValueError('secret exceptions must be a list')
    seen = set()
    for entry in entries:
        extra = ({'history_commits'} if 'history_commits' in entry else set()) | \
                ({'code_evidence'} if entry.get('kind') == 'inert-historical-credential' else set())
        if set(entry) != set(IDENTITY) | {'kind', 'owner', 'expires', 'rationale', 'provenance'} | extra:
            raise ValueError('invalid secret disposition fields')
        if entry['kind'] not in KINDS or entry['path'] != normalized_path(entry['path']):
            raise ValueError('unreviewed kind or non-normalized exception path')
        if not re.fullmatch(r'[a-z0-9-]+', entry['rule']):
            raise ValueError('invalid detector rule')
        for field in ('match_sha256', 'value_sha256', 'content_sha256'):
            if not re.fullmatch(r'[0-9a-f]{64}', entry[field]):
                raise ValueError('exception must bind exact reviewed content')
        commits = entry.get('history_commits')
        if commits is not None and (not isinstance(commits, list) or not commits or len(set(commits)) != len(commits) or
                                    not all(isinstance(c, str) and re.fullmatch('[0-9a-f]{40}', c) for c in commits)):
            raise ValueError('history-only disposition requires exact immutable commits')
        if date.fromisoformat(entry['expires']) > today + timedelta(days=366 if commits else 31):
            raise ValueError('secret disposition exceeds historical/current-tree expiry cadence')
        if entry['kind'] == 'inert-historical-credential' and (
                not commits or entry.get('code_evidence') != INERT_DEVICE_REFRESH_EVIDENCE or
                entry['rule'] != 'generic-api-key' or entry['path'] not in
                {'docs/device-code-binding.md', 'skills/nyxid/references/devices.md'} or
                entry['value_sha256'] != INERT_DEVICE_REFRESH_HASH):
            raise ValueError('inert historical kind requires the exact reviewed device-token identity and code evidence')
        if date.fromisoformat(entry['expires']) <= today:
            raise ValueError('expired secret exception: ' + entry['path'] + ' (' + entry['rule'] + ')')
        for field in ('owner', 'rationale', 'provenance'):
            if not isinstance(entry[field], str) or not entry[field].strip():
                raise ValueError('exception requires owner, rationale and provenance')
        identity = tuple(entry[key] for key in IDENTITY)
        if identity in seen:
            raise ValueError('duplicate secret disposition')
        seen.add(identity)
    return entries


def finding_identity(finding, content):
    """Bind full original source lines as well as the detector match and value.

    A second key even on the same source line changes content_sha256. Nothing
    matches by file, detector, commit fingerprint or secret hash alone.
    """
    start, end = finding['StartLine'], finding['EndLine']
    lines = content.splitlines(keepends=True)
    if type(start) is not int or type(end) is not int or not 1 <= start <= end <= len(lines):
        raise ValueError('scanner source range is invalid')
    original = b''.join(lines[start - 1:end])
    if finding['Secret'].encode() not in original or finding['Match'].encode() not in original:
        raise ValueError('scanner finding does not match its original source lines')
    return dict(rule=finding['RuleID'], path=normalized_path(finding['File']),
                match_sha256=digest(finding['Match'].encode()),
                value_sha256=digest(finding['Secret'].encode()),
                content_sha256=digest(original))


def disposition(identity, entries, *, scope='tree', commit=None):
    return next((entry for entry in entries if all(entry[key] == identity[key] for key in IDENTITY)
                 and ('history_commits' not in entry or
                      (scope == 'history' and commit in entry['history_commits']))), None)
