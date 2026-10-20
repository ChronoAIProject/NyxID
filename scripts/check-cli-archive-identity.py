#!/usr/bin/env python3
"""Require retained identity from the actual stripped CLI archive member."""
import argparse
from pathlib import Path
import tempfile

from publication_contract import load_json, validate_plan
from source_artifacts import archives, verify_binary


def check(plan, artifacts, extractor, target):
    validate_plan(plan)
    subjects = [item for item in plan['subjects'] if item.get('artifact_kind') == 'executable-zip' and item.get('target') == target]
    if len(subjects) != 1:
        raise ValueError('exactly one planned CLI target is required')
    subject = subjects[0]
    with tempfile.TemporaryDirectory(prefix='nyxid-archive-identity-') as temporary:
        directory = Path(temporary)
        inspection = archives.inspect_archive(artifacts / subject['name'], subject, directory)
        verify_binary(directory / subject['binary'], extractor, subject['package'], plan['version'], target,
                      plan['source']['commit'], plan['source']['epoch'])
        print(f'Final CLI archive source identity verified: {target}, {inspection["digest"]}, SOURCE_COMMIT={plan["source"]["commit"]}')


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--plan', type=Path, required=True)
    parser.add_argument('--artifacts', type=Path, required=True)
    parser.add_argument('--extractor', type=Path, required=True)
    parser.add_argument('--target', required=True)
    args = parser.parse_args()
    check(load_json(args.plan), args.artifacts, args.extractor.resolve(), args.target)


if __name__ == '__main__':
    main()
