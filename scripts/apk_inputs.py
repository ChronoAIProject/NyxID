"""APK catalog/caller admission and exact package-stage projections."""
import re
import subprocess

from apt_inputs import stages
from resolve_apk_requests import REQUESTS

INSTALLER = "sh /opt/nyxid-apk/apk-input-install.sh"
INPUTS = ("scripts/apk-input-install.sh", "scripts/apk-catalog.awk", "scripts/apk-inventory.awk", "security/apk/")
ARCHES = {"amd64": "x86_64", "arm64": "aarch64"}


def catalog(root, path, base, arch):
    result = subprocess.run(["awk", "-v", "base=" + base, "-v", "architecture=" + arch,
                             "-f", str(root / INPUTS[1]), str(path)], text=True, capture_output=True)
    if result.returncode:
        raise ValueError(result.stderr.strip())
    return result.stdout


def check_apk_inputs(root, dockerfiles):
    errors, consumed = [], set()
    for name in dockerfiles:
        for stage in stages((root / name).read_text()):
            before, calls = "", []
            for raw in stage.body:
                if raw.startswith("RUN "):
                    if re.search(r"(?<![\w-])(?:/sbin/)?apk\s", raw):
                        errors.append(name + ": raw APK bypasses archive installer")
                    if "apk-input-install.sh" in raw:
                        matches = re.findall(re.escape(INSTALLER) + r" (--download|--install) (/opt/nyxid-apk/catalogs/([a-z0-9-]+)-\$\{TARGETARCH\}\.tsv) (\S+) /opt/nyxid-apk/archives\s*$", raw)
                        if len(matches) != 1 or raw.count("apk-input-install.sh") != 1:
                            errors.append(name + ": unsupported APK invocation")
                        else:
                            mode, _, label, base = matches[0]
                            if mode == "--install" and not raw.startswith("RUN --network=none "):
                                errors.append(name + ": APK install must have network=none")
                            if base != stage.base:
                                errors.append(name + ": APK caller base mismatch")
                            if any(value not in before for value in INPUTS) or "ARG TARGETARCH" not in before:
                                errors.append(name + ": APK stage omits installer/parser/catalog/architecture")
                            calls.append((mode, label))
                before += raw
            if calls:
                label = calls[0][1]
                if calls != [("--download", label), ("--install", label)] or label not in REQUESTS or REQUESTS[label][:2] != (name, stage.name):
                    errors.append(name + ": unexpected APK stage/request binding")
                    continue
                consumed.add(label)
                for arch, native in ARCHES.items():
                    path = root / "security/apk" / (label + "-" + arch + ".tsv")
                    try:
                        catalog(root, path, stage.base, native)
                        packages = next(line.removeprefix("# packages ").split() for line in path.read_text().splitlines() if line.startswith("# packages "))
                        if packages != sorted(REQUESTS[label][2]):
                            errors.append(str(path) + ": requested packages differ from reviewed stage")
                    except (OSError, ValueError, StopIteration) as error:
                        errors.append(str(path) + ": " + str(error))
    if consumed != set(REQUESTS):
        errors.append("APK catalog coverage missing stages: " + str(sorted(set(REQUESTS) - consumed)))
    expected = {label + "-" + arch + ".tsv" for label in REQUESTS for arch in ARCHES}
    if {p.name for p in (root / "security/apk").glob("*.tsv")} != expected:
        errors.append("APK catalog matrix has missing or unexpected inputs")
    return errors


def prefixes(root, dockerfiles):
    result = []
    for name in dockerfiles:
        for stage in stages((root / name).read_text()):
            ends = [i for i, raw in enumerate(stage.body) if INSTALLER + " --install " in raw]
            if ends:
                result.append((name, stage.name, "".join(stage.arguments + stage.body[:max(ends) + 1])))
    return result
