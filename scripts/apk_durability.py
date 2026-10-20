"""Availability evidence for repository-owned archives; no package-source fallback."""
import json
import os
import urllib.error
import urllib.request

from apk_archive_release import expected_assets
from apk_inputs import ARCHES

API = 'https://api.github.com/repos/ChronoAIProject/NyxID'


class NoRedirect(urllib.request.HTTPRedirectHandler):
    def redirect_request(self, request, fp, code, message, headers, url):
        raise ValueError('durability API redirected; refusing off-endpoint response')


def get_json(path):
    headers = {'Accept': 'application/vnd.github+json', 'User-Agent': 'NyxID-build-input-durability',
               'X-GitHub-Api-Version': '2022-11-28'}
    if os.environ.get('GH_TOKEN'):
        headers['Authorization'] = 'Bearer ' + os.environ['GH_TOKEN']
    request = urllib.request.Request(API + path, headers=headers)
    with urllib.request.build_opener(NoRedirect()).open(request, timeout=30) as response:
        return json.load(response)


def check(catalogs, fetch=get_json):
    tag, expected = expected_assets(catalogs, ARCHES)
    try:
        release = fetch('/releases/tags/' + tag)
    except urllib.error.HTTPError as error:
        if error.code == 404:
            return [tag + ': repository archive release is missing']
        raise
    if release.get('draft') or release.get('tag_name') != tag:
        return [tag + ': archive release is not publicly available under the catalog tag']
    found, errors = {}, []
    for page in range(1, 101):
        assets = fetch('/releases/' + str(int(release['id'])) + '/assets?per_page=100&page=' + str(page))
        for asset in assets:
            name = asset['name']
            if name in found:
                errors.append(tag + ': duplicate asset ' + name)
            found[name] = asset
        if len(assets) < 100:
            break
    else:
        raise ValueError('archive asset pagination exceeded bound')
    for name, checksum in expected.items():
        asset = found.get(name)
        if not asset:
            errors.append(tag + ': missing asset ' + name)
        elif asset.get('state') != 'uploaded' or asset.get('size', 0) <= 0 or asset.get('digest') != 'sha256:' + checksum:
            errors.append(tag + ': incomplete or incorrectly bound asset ' + name)
    return errors
