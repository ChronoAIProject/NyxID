"""APK catalog admission and exact package-stage projections."""
import re
import subprocess

from apt_inputs import stages
from resolve_apk_requests import REQUESTS

INSTALLER = "sh /opt/nyxid-apk/apk-input-install.sh"
INPUTS = ("scripts/apk-input-install.sh", "scripts/apk-catalog.awk", "scripts/apk-inventory.awk", "security/apk/")
ARCHES = {"amd64": "x86_64", "arm64": "aarch64"}
ARCHIVES = "/opt/nyxid-apk/archives"


def catalog(root, path, base, arch):
    result = subprocess.run(["awk", "-v", "base=" + base, "-v", "architecture=" + arch,
                             "-f", str(root / INPUTS[1]), str(path)], text=True, capture_output=True)
    if result.returncode:
        raise ValueError(result.stderr.strip())
    return result.stdout


def check_apk_inputs(root, dockerfiles):
    errors, consumed = [], set()
    for name in dockerfiles:
        parsed = stages((root / name).read_text())
        aliases = {s.name: s for s in parsed}
        calls = {}
        for stage in parsed:
            before = ""
            for raw in stage.body:
                if raw.startswith("RUN "):
                    if re.search(r"(?<![\w-])(?:/sbin/)?apk\s", raw):
                        errors.append(name + ": raw APK bypasses archive installer")
                    if "apk-input-install.sh" in raw:
                        match = re.search(re.escape(INSTALLER) + r' (--download|--install) /opt/nyxid-apk/catalogs/([a-z0-9-]+)-\$\{TARGETARCH\}\.tsv (\S+) ' + ARCHIVES + r'( "\$\{APK_INPUT_SOURCE\}")?\s*$', raw)
                        if not match or raw.count("apk-input-install.sh") != 1:
                            errors.append(name + ": unsupported APK invocation")
                        else:
                            mode, label, base, source = match.groups()
                            calls.setdefault(stage.name, []).append((mode, label))
                            if base != stage.base:
                                errors.append(name + ": APK caller base mismatch")
                            if any(value not in before for value in INPUTS) or "ARG TARGETARCH" not in before:
                                errors.append(name + ": APK stage omits installer/parser/catalog/architecture")
                            if mode == "--download":
                                if not source or "ARG APK_INPUT_SOURCE=durable\n" not in before or "scripts/apk-input-fetch.sh" not in before:
                                    errors.append(name + ": APK download must default to durable source")
                            else:
                                mount = "--mount=type=bind,from=" + label + "-apk-download,source=" + ARCHIVES + ",target=" + ARCHIVES
                                if not raw.startswith("RUN --network=none "):
                                    errors.append(name + ": APK install must have network=none")
                                expected = "RUN --network=none " + mount + " " + INSTALLER + " --install /opt/nyxid-apk/catalogs/" + label + "-${TARGETARCH}.tsv " + base + " " + ARCHIVES
                                if " ".join(raw.replace("\\\n", " ").split()) != expected:
                                    errors.append(name + ": APK install requires a read-only download-stage mount")
                before += raw
        for stage in parsed:
            stage_calls = calls.get(stage.name, [])
            if len(stage_calls) > 1:
                errors.append(name + ": APK download and install must use separate stages")
            if not stage_calls:
                continue
            mode, label = stage_calls[0]
            if label not in REQUESTS:
                errors.append(name + ": unexpected APK stage/request binding")
                continue
            expected_name = label + "-apk-download" if mode == "--download" else REQUESTS[label][1]
            if name != REQUESTS[label][0] or stage.name != expected_name:
                errors.append(name + ": unexpected APK stage/request binding")
            if mode == "--install":
                consumed.add(label)
                downloader = aliases.get(label + "-apk-download")
                if not downloader or calls.get(downloader.name) != [("--download", label)] or downloader.base != stage.base:
                    errors.append(name + ": missing matching APK download stage")
                for arch, native in ARCHES.items():
                    path = root / "security/apk" / (label + "-" + arch + ".tsv")
                    try:
                        catalog(root, path, stage.base, native)
                        packages = next(line.removeprefix("# packages ").split() for line in path.read_text().splitlines() if line.startswith("# packages "))
                        if packages != sorted(REQUESTS[label][2]):
                            errors.append(str(path) + ": requested packages differ from reviewed stage")
                    except (OSError, ValueError, StopIteration) as error:
                        errors.append(str(path) + ": " + str(error))
        downloaders = {s.name for s in parsed if any(mode == "--download" for mode, _ in calls.get(s.name, []))}
        for stage in parsed:
            if stage.base in downloaders or any(raw.startswith("COPY ") and any("--from=" + d in raw for d in downloaders) for raw in stage.body):
                errors.append(name + ": APK download layers must not be inherited or copied")
        if parsed and parsed[-1].name in downloaders:
            errors.append(name + ": APK download stage cannot be the final image")
    if consumed != set(REQUESTS):
        errors.append("APK catalog coverage missing stages: " + str(sorted(set(REQUESTS) - consumed)))
    expected = {label + "-" + arch + ".tsv" for label in REQUESTS for arch in ARCHES}
    if {p.name for p in (root / "security/apk").glob("*.tsv")} != expected:
        errors.append("APK catalog matrix has missing or unexpected inputs")
    return errors


def prefixes(root, dockerfiles):
    result = []
    for name in dockerfiles:
        parsed = stages((root / name).read_text())
        aliases = {s.name: s for s in parsed}
        for stage in parsed:
            ends = [i for i, raw in enumerate(stage.body) if INSTALLER + " --install " in raw]
            if ends:
                prefix = stage.body[:max(ends) + 1]
                dependency = re.search(r"--mount=type=bind,from=([^,]+)", "".join(prefix)).group(1)
                result.append((name, stage.name, "".join(stage.arguments + aliases[dependency].body + prefix)))
    return result
