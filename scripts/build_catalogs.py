"""Validate frozen APT/APK/CA inputs and generate unaccepted HTTPS candidates."""
from datetime import datetime, timedelta, timezone
import hashlib
from html.parser import HTMLParser
from pathlib import Path
import re
import ssl
import subprocess
import tempfile
import urllib.parse
import urllib.request

from apt_inputs import ROOTS, select
from apk_archive_release import expected_assets
from apk_inputs import ARCHES

ROOT = Path(__file__).resolve().parents[1]
MAX_AGE_DAYS = 14  # weekly refresh plus one missed weekly run
SNAPSHOT_SETTLE_DAYS = 2  # do not select an active/recent archive import
MAX_CA_SOURCE_AGE_DAYS = 90  # upstream publishes bundles less often than weekly
CA_URL = 'https://curl.se/ca/cacert.pem'
CA_CHECKSUM_URL = CA_URL + '.sha256'
CA_HEADER = '# nyxid.apt-transport-ca.v1'


class SnapshotRedirect(urllib.request.HTTPRedirectHandler):
    def redirect_request(self, request, fp, code, message, headers, url):
        before, after = urllib.parse.urlsplit(request.full_url), urllib.parse.urlsplit(url)
        if after.scheme != 'https' or after.netloc != before.netloc or after.username or after.password:
            raise ValueError('candidate redirect changed HTTPS origin')
        return super().redirect_request(request, fp, code, message, headers, url)


def fetch_https(url, origins, maximum, *, expected_query=''):
    parsed = urllib.parse.urlsplit(url)
    if (parsed.scheme != 'https' or parsed.netloc not in origins or parsed.username or parsed.password or
            parsed.query != expected_query or parsed.fragment):
        raise ValueError('candidate requires an official HTTPS origin')
    with urllib.request.build_opener(SnapshotRedirect()).open(url, timeout=120) as response:
        data = response.read(maximum + 1)
    if len(data) > maximum:
        raise ValueError('candidate exceeds download size bound')
    return data


def fetch(url):
    data = fetch_https(url, {'snapshot.debian.org', 'snapshot.ubuntu.com'}, 4 * 1024 * 1024)
    if not data.startswith(b'-----BEGIN PGP SIGNED MESSAGE-----') or b'-----BEGIN PGP SIGNATURE-----' not in data:
        raise ValueError('candidate InRelease is absent or unsigned')
    return data


def fetch_debian_listing(archive, year, month):
    if archive not in {'debian', 'debian-security'} or not 1 <= month <= 12 or not 2005 <= year <= 9999:
        raise ValueError('invalid Debian archive listing request')
    query = f'year={year}&month={month}'
    return fetch_https(f'https://snapshot.debian.org/archive/{archive}/?{query}',
                       {'snapshot.debian.org'}, 1024 * 1024, expected_query=query)


def snapshot_date(stamp):
    if not re.fullmatch(r'[0-9]{8}T[0-9]{6}Z', stamp):
        raise ValueError('non-canonical snapshot timestamp')
    return datetime.strptime(stamp, '%Y%m%dT%H%M%SZ').replace(tzinfo=timezone.utc)


class DebianRuns(HTMLParser):
    """Only relative canonical run links from the requested monthly listing."""
    def __init__(self, year, month):
        super().__init__()
        self.month = (year, month)
        self.runs = set()

    def handle_starttag(self, tag, attrs):
        if tag != 'a':
            return
        for name, value in attrs:
            if name == 'href' and value and re.fullmatch(r'[0-9]{8}T[0-9]{6}Z/', value):
                stamp = snapshot_date(value[:-1])
                if (stamp.year, stamp.month) != self.month or stamp in self.runs:
                    raise ValueError('invalid or duplicate Debian archive run')
                self.runs.add(stamp)


def settled_snapshots(before=None, now=None, listing=fetch_debian_listing):
    now = now or datetime.now(timezone.utc)
    settled = now - timedelta(days=SNAPSHOT_SETTLE_DAYS)
    cutoff = snapshot_date(before) if before is not None else settled.replace(microsecond=0)
    if cutoff > settled or now - cutoff > timedelta(days=MAX_AGE_DAYS):
        raise ValueError('snapshot cutoff must be at least 48 hours old and within the freshness window')
    # Include adjacent months for month/year boundaries and a later completed
    # import. Never treat a rounded cutoff itself as a Debian archive run.
    months = sorted({(date.year, date.month) for date in
                     (cutoff - timedelta(days=MAX_AGE_DAYS), cutoff, now)})
    result = {'ubuntu': cutoff.strftime('%Y%m%dT%H%M%SZ')}
    for archive in ('debian', 'debian-security'):
        runs = set()
        for year, month in months:
            parser = DebianRuns(year, month)
            parser.feed(listing(archive, year, month).decode('utf-8'))
            runs.update(parser.runs)
        runs = sorted(date for date in runs if date <= now)
        eligible = [date for date in runs[:-1] if now - timedelta(days=MAX_AGE_DAYS) <= date <= cutoff]
        if not eligible:
            raise ValueError(f'no settled, listed {archive} run within the freshness window')
        result[archive] = max(eligible).strftime('%Y%m%dT%H%M%SZ')
    return result


def fetch_ca(url):
    if url not in {CA_URL, CA_CHECKSUM_URL}:
        raise ValueError('CA source must be the exact official bundle/checksum URL')
    return fetch_https(url, {'curl.se'}, 512 * 1024)


def source_stamp(data):
    match = re.search(rb'^## Certificate data from Mozilla as of: (.+ GMT)$', data, re.M)
    if not match:
        raise ValueError('Mozilla source timestamp missing from CA bundle')
    return datetime.strptime(match[1].decode('ascii'), '%a %b %d %H:%M:%S %Y GMT').replace(tzinfo=timezone.utc)


def ca_provenance(pins, bundle):
    if pins.is_symlink() or bundle.is_symlink():
        raise ValueError('CA provenance and bundle must be regular files')
    result = subprocess.run(['awk', '-f', str(ROOT / 'scripts/apt-transport-ca.awk'), str(pins)],
                            capture_output=True, text=True)
    if result.returncode:
        raise ValueError('invalid CA provenance manifest')
    source, checked, url, checksum_url, expected = pins.read_text().splitlines()[1].split('\t')
    source_date = datetime.strptime(source, '%Y-%m-%dT%H:%M:%SZ').replace(tzinfo=timezone.utc)
    checked_date = datetime.strptime(checked, '%Y-%m-%d').replace(tzinfo=timezone.utc)
    if source_date.strftime('%Y-%m-%dT%H:%M:%SZ') != source or checked_date.strftime('%Y-%m-%d') != checked:
        raise ValueError('non-canonical CA provenance dates')
    data = bundle.read_bytes()
    if len(data) > 512 * 1024 or hashlib.sha256(data).hexdigest() != expected:
        raise ValueError('CA bundle differs from pins manifest')
    if source_stamp(data) != source_date or source_date.date() > checked_date.date():
        raise ValueError('CA provenance differs from upstream bundle timestamp')
    if data.count(b'-----BEGIN CERTIFICATE-----') < 100:
        raise ValueError('CA bundle is incomplete')
    # Parse every certificate without installing any of them into system trust.
    ssl.SSLContext(ssl.PROTOCOL_TLS_CLIENT).load_verify_locations(cafile=str(bundle))
    return source_date, checked_date


def dates(apt, apk):
    selected = select(ROOT, ROOTS, apt)
    result = {}
    for row in selected.splitlines():
        distro, codename, _, _, url, _ = row.split('\t')
        label = 'APT ' + distro + ':' + codename
        stamp = snapshot_date(url.rsplit('/', 1)[1])
        result[label] = min(result.get(label, stamp), stamp)
    tag, _ = expected_assets(apk, ARCHES)
    result['APK catalogs'] = datetime.strptime(tag.removeprefix('build-inputs-apk-'), '%Y-%m-%d').replace(tzinfo=timezone.utc)
    return result


def freshness(apt, apk, ca_pins, ca_bundle, now=None):
    now = now or datetime.now(timezone.utc)
    stamps = dates(apt, apk)
    # The oldest archive controls staleness, but every suite must also be in
    # the past. A newer/future security run must not hide behind an old base.
    for row in select(ROOT, ROOTS, apt).splitlines():
        distro, codename, _, _, url, _ = row.split('\t')
        stamp = snapshot_date(url.rsplit('/', 1)[1])
        if stamp > now:
            label = 'APT ' + distro + ':' + codename
            stamps[label] = max(stamps[label], stamp)
    source, checked = ca_provenance(ca_pins, ca_bundle)
    stamps['APT transport CA upstream check'] = checked
    errors = []
    for label, stamp in stamps.items():
        age = now - stamp
        if age < timedelta(0):
            errors.append(f'{label}: input date is in the future ({stamp.date()})')
        elif age > timedelta(days=MAX_AGE_DAYS):
            errors.append(f'{label}: stale input {stamp.date()} exceeds {MAX_AGE_DAYS} days; refresh frozen security inputs')
    if source > now:
        errors.append('APT transport CA: upstream source timestamp is in the future')
    elif now - source > timedelta(days=MAX_CA_SOURCE_AGE_DAYS):
        errors.append(f'APT transport CA: stale upstream bundle exceeds {MAX_CA_SOURCE_AGE_DAYS} days')
    return errors


def refresh_ca(output, checked, download=fetch_ca):
    checked_date = datetime.strptime(checked, '%Y-%m-%d').replace(tzinfo=timezone.utc)
    if checked_date.strftime('%Y-%m-%d') != checked:
        raise ValueError('non-canonical CA refresh date')
    data, checksum = download(CA_URL), download(CA_CHECKSUM_URL)
    if not re.fullmatch(rb'[0-9a-f]{64}\s+(?:\*)?cacert\.pem\s*', checksum):
        raise ValueError('invalid upstream CA checksum publication')
    expected = checksum.decode('ascii').split()[0]
    if hashlib.sha256(data).hexdigest() != expected:
        raise ValueError('upstream CA checksum mismatch')
    source = source_stamp(data).strftime('%Y-%m-%dT%H:%M:%SZ')
    manifest = CA_HEADER + '\n' + '\t'.join((source, checked, CA_URL, CA_CHECKSUM_URL, expected)) + '\n'
    with tempfile.TemporaryDirectory(prefix='nyxid-ca-candidate-') as temporary:
        temporary = Path(temporary)
        (temporary / 'ca.pem').write_bytes(data)
        (temporary / 'ca.tsv').write_text(manifest)
        ca_provenance(temporary / 'ca.tsv', temporary / 'ca.pem')
    output.mkdir(parents=True, exist_ok=True)
    (output / 'apt-transport-ca.pem').write_bytes(data)
    (output / 'apt-transport-ca.tsv').write_text(manifest)
    return manifest


def refresh_apt(output, before=None, download=fetch, *, now=None, listing=fetch_debian_listing):
    # HTTPS hashes alone never accept a candidate. Native replay with distro
    # keyrings must authenticate its signatures/indexes/packages before a PR.
    stamps = settled_snapshots(before, now, listing)
    rows = select(ROOT, ROOTS).splitlines()
    refreshed = ['# distro codename suite components snapshot-url signed-InRelease-sha256']
    for row in rows:
        fields = row.split('\t')
        archive = 'ubuntu' if fields[0] == 'ubuntu' else ('debian-security' if fields[2].endswith('-security') else 'debian')
        fields[4] = fields[4].rsplit('/', 1)[0] + '/' + stamps[archive]
        metadata = download(fields[4] + '/dists/' + fields[2] + '/InRelease')
        fields[5] = hashlib.sha256(metadata).hexdigest()
        refreshed.append('\t'.join(fields))
    text = '\n'.join(refreshed) + '\n'
    with tempfile.TemporaryDirectory(prefix='nyxid-apt-candidate-') as temporary:
        candidate = Path(temporary) / 'apt-snapshots.tsv'
        candidate.write_text(text)
        select(ROOT, ROOTS, candidate)
    output.parent.mkdir(parents=True, exist_ok=True)
    output.write_text(text)
    return text
