#!/usr/bin/env python3
"""Pair two saved-login controls with the candidate, idle and publishing.

Supply the base and candidate nyxid-server test executables. Set
NYXID_TEST_DATABASE_URL to the dedicated replica set. Each invocation uses a
separate database. Write stdout to JSONL to retain the individual rounds.
"""

import argparse
import json
import os
import random
import re
import subprocess
import sys


def run(binary, mode, label, block):
    env = dict(os.environ, NYXID_CENSUS_LOAD_MODE=str(mode), RUST_MIN_STACK="8388608")
    result = subprocess.run(
        [binary, "saved_login_writes_during_capture", "--ignored", "--nocapture", "--test-threads=1"],
        env=env,
        stdout=subprocess.PIPE,
        stderr=subprocess.STDOUT,
        text=True,
    )
    if result.returncode:
        sys.exit(result.stdout)
    line, = (line for line in result.stdout.splitlines() if "CENSUS_LOAD round=" in line)
    sample = {name: int(value) for name, value in re.findall(r"(\w+)=(\d+)", line)}
    if (sample["completed"] != 400 or sample["errors"] or sample["foreground_journal"]
            or (mode == 2 and not sample["cuts"])):
        sys.exit(f"incomplete sample {sample}")
    sample.update(round=block, label=label, load_avg=os.getloadavg())
    print(json.dumps(sample), flush=True)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("base")
    parser.add_argument("candidate")
    parser.add_argument("--rounds", type=int, default=8)
    args = parser.parse_args()
    if args.rounds <= 0 or args.rounds % 4:
        parser.error("rounds must be a positive multiple of four for position balance")
    if not os.environ.get("NYXID_TEST_DATABASE_URL"):
        parser.error("set NYXID_TEST_DATABASE_URL to the test replica set")
    arms = [(args.base, 0), (args.base, 1), (args.candidate, 0), (args.candidate, 2)]
    run(*arms[0], 0, -1)
    run(*arms[3], 3, -1)
    rng = random.Random(195507)
    for group in range(args.rounds // 4):
        order = rng.sample(range(4), 4)
        for i, offset in enumerate(rng.sample(range(4), 4)):
            for label in order[offset:] + order[:offset]:
                run(*arms[label], label, group * 4 + i)


if __name__ == "__main__":
    main()
