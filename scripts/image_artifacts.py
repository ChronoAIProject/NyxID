"""Read final image bytes without starting the image or trusting archive paths."""
from contextlib import contextmanager
import json
from pathlib import Path, PurePosixPath
import re
import subprocess
import tarfile


def unpack(stream, destination, limit=1024 * 1024 * 1024):
    destination.mkdir(parents=True, exist_ok=False)
    seen, files, total = set(), [], 0
    with tarfile.open(fileobj=stream, mode='r|') as archive:
        for member in archive:
            name = member.name.removeprefix('./').rstrip('/')
            if name in ('', '.') and member.isdir():
                continue
            parts = PurePosixPath(name).parts
            if not parts or name != '/'.join(parts) or any(part in ('.', '..') for part in parts) or \
                    name.startswith('/') or '\\' in name or any(ord(c) < 32 for c in name) or name in seen:
                raise ValueError('unsafe or duplicate image archive path')
            seen.add(name)
            if len(seen) > 100000 or not (member.isdir() or member.isfile()):
                raise ValueError('image archive contains links, special files or too many members')
            path = destination.joinpath(*parts)
            if member.isdir():
                path.mkdir(parents=True, exist_ok=True)
                continue
            total += member.size
            if total > limit:
                raise ValueError('image archive exceeds size bound')
            path.parent.mkdir(parents=True, exist_ok=True)
            with archive.extractfile(member) as source, path.open('xb') as output:
                remaining = member.size
                while remaining:
                    block = source.read(min(1024 * 1024, remaining))
                    if not block:
                        raise ValueError('truncated image archive')
                    output.write(block)
                    remaining -= len(block)
            files.append(name)
    return files


@contextmanager
def image_container(reference):
    info = json.loads(subprocess.check_output(['docker', 'image', 'inspect', reference]))
    if len(info) != 1 or not re.fullmatch('sha256:[0-9a-f]{64}', info[0]['Id']):
        raise ValueError('expected one immutable local image')
    # Resolve the tag once; every extraction uses this same immutable image ID.
    container = subprocess.check_output(['docker', 'create', '--network=none',
                                         '--entrypoint=/bin/true', info[0]['Id']], text=True).strip()
    try:
        yield container, info[0]
    finally:
        subprocess.run(['docker', 'rm', '-v', container], check=True, stdout=subprocess.DEVNULL)


def copy(container, path, destination):
    with subprocess.Popen(['docker', 'cp', container + ':' + path, '-'], stdout=subprocess.PIPE) as child:
        try:
            files = unpack(child.stdout, destination)
        except BaseException:
            child.terminate()
            raise
        finally:
            child.stdout.close()
        if child.wait() != 0:
            raise ValueError('image artifact extraction failed: ' + path)
    return files
