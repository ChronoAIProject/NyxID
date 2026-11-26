#!/usr/bin/env python3
"""One public web artifact identity for all architectures of a publish run."""
import os
import uuid

identity = ":".join(os.environ[key] for key in
                    ("GITHUB_REPOSITORY", "GITHUB_SHA", "GITHUB_RUN_ID"))
print(uuid.uuid5(uuid.NAMESPACE_URL, f"https://github.com/{identity}"))
