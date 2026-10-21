"""Publication-only OCI SBOM attachment (never called by PR generation/replay)."""
import re
import subprocess


def repository_check(repository):
    if not re.fullmatch(r'ghcr.io/[a-z0-9_-]+/nyxid/[a-z0-9_-]+', repository):
        raise ValueError('reviewed GHCR application repository required')


def attach(regctl, subject, sbom):
    subprocess.run([str(regctl), 'artifact', 'put', '--artifact-type', 'application/vnd.cyclonedx+json',
                    '--subject', subject, '--file', str(sbom), '--file-media-type', 'application/vnd.cyclonedx+json',
                    '--by-digest'], check=True)
