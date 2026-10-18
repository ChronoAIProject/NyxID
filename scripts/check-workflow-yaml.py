#!/usr/bin/env python3
from pathlib import Path
from workflow_yaml import load_unique, validate_rust_steps

root = Path(__file__).resolve().parents[1]
errors = []
paths = sorted(path for path in (root / ".github/workflows").iterdir()
               if path.suffix in (".yml", ".yaml"))
rust_steps = 0
for path in paths:
    workflow = load_unique(path.read_text())
    errors.extend(str(path.relative_to(root)) + ": " + error for error in validate_rust_steps(workflow))
    rust_steps += sum(step.get("uses", "").startswith("dtolnay/rust-toolchain@")
                      for job in workflow.get("jobs", {}).values()
                      for step in job.get("steps", []))
load_unique((root / ".github/dependabot.yml").read_text())
if errors:
    raise SystemExit("\n".join(errors))
print(f"{len(paths)} workflows and Dependabot have unique mappings; "
      f"all {rust_steps} pinned Rust Actions explicitly select toolchain 1.98.1")
