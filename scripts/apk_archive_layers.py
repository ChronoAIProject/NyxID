"""Reject archive bytes in any saved image layer, including later-deleted files."""
import json
from pathlib import PurePosixPath
import tarfile


def check_layers(path):
    count = 0
    with tarfile.open(path) as image:
        manifests = json.load(image.extractfile('manifest.json'))
        for manifest in manifests:
            for name in manifest['Layers']:
                with tarfile.open(fileobj=image.extractfile(name), mode='r|*') as layer:
                    for entry in layer:
                        parts = PurePosixPath(entry.name).parts
                        if entry.isdir():
                            continue
                        normalized = '/'.join(p for p in parts if p != '.')
                        if normalized.startswith('opt/nyxid-apk/archives/') or normalized.endswith('.apk'):
                            raise ValueError('APK archive payload retained in image layer: ' + name + ':' + entry.name)
                count += 1
    if not count:
        raise ValueError('saved image has no layers')
    return count
