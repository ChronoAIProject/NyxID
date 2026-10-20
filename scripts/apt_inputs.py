"""APT catalog admission and exact Dockerfile package-layer projections."""
from dataclasses import dataclass
from pathlib import Path
import re
import subprocess

ROOTS = ("debian:bookworm", "debian:trixie", "ubuntu:focal", "ubuntu:noble")
INSTALLER = "sh /opt/nyxid-apt/apt-snapshot-install.sh /opt/nyxid-apt/apt-snapshots.tsv"
INPUTS = ("scripts/apt-snapshot-install.sh", "scripts/apt-snapshot-select.awk", "security/apt-snapshots.tsv", "security/apt-transport-ca.pem")
RAW_APT = re.compile(r"(?<![\w-])(?:/usr/bin/)?apt(?:-get)?(?=\s|$)")


def select(root, selectors, catalog=None):
    result = subprocess.run(["awk", "-v", "selectors=" + " ".join(selectors), "-f",
                             str(root / INPUTS[1]), str(catalog or root / INPUTS[2])],
                            capture_output=True, text=True)
    if result.returncode:
        raise ValueError(result.stderr.strip())
    return result.stdout


def instructions(text):
    """Keep original instruction bytes, including continuations and heredocs."""
    lines = iter(text.splitlines(keepends=True))
    for line in lines:
        if not line.strip() or line.lstrip().startswith("#"):
            continue
        raw = line
        while raw.rstrip().endswith("\\"):
            raw += next(lines)
        ends = re.findall(r"<<-?['\"]?([A-Za-z_][A-Za-z_0-9]*)['\"]?", raw)
        for end in ends:
            for line in lines:
                raw += line
                if line.strip() == end:
                    break
            else:
                raise ValueError("unterminated Dockerfile heredoc")
        yield raw


@dataclass
class Stage:
    name: str
    base: str
    body: list
    arguments: list

    def apt_prefix(self):
        return self.body[:max(i for i, value in enumerate(self.body) if INSTALLER in value) + 1]


def stages(text):
    result, arguments = [], []
    for raw in instructions(text):
        match = re.fullmatch(r"FROM\s+(\S+)(?:\s+AS\s+(\S+))?\s*", raw, re.I)
        if match:
            base, name = match.groups()
            result.append(Stage(name or str(len(result)), base, [raw], list(arguments)))
        elif result:
            result[-1].body.append(raw)
        elif raw.startswith("ARG "):
            arguments.append(raw)
        else:
            raise ValueError("unsupported instruction before FROM")
    return result


def projection(stage, all_stages):
    prefix = stage.apt_prefix()
    aliases = {s.name: s for s in all_stages}
    base = stage.base
    visited = set()
    while base in aliases:
        if base in visited or len(aliases[base].body) != 1:
            raise ValueError("APT projection requires the full inherited stage: " + base)
        visited.add(base)
        base = aliases[base].base
    if base != stage.base:
        prefix = [prefix[0].replace(stage.base, base, 1)] + prefix[1:]
    # No changes to any package instruction, COPY, ENV, ARG or preference setup.
    return "".join(stage.arguments + prefix)


def check_apt_inputs(root, dockerfiles):
    errors = []
    try:
        select(root, ROOTS)
    except (ValueError, OSError) as error:
        errors.append("APT snapshot catalog: " + str(error))
    for name in dockerfiles:
        try:
            for stage in stages((root / name).read_text()):
                previous = ""
                for raw in stage.body:
                    if raw.lstrip().startswith("RUN "):
                        flat = raw.replace("\\\n", " ")
                        if RAW_APT.search(flat):
                            errors.append(name + ": raw APT command bypasses snapshot installer")
                        if "apt-snapshot-install.sh" in flat:
                            calls = re.findall(re.escape(INSTALLER) + r" ((?:(?:debian|ubuntu):[a-z]+\s+)+)--\s+", flat)
                            if len(calls) != flat.count("apt-snapshot-install.sh"):
                                errors.append(name + ": unsupported APT snapshot invocation")
                            for call in calls:
                                try:
                                    select(root, call.split())
                                except ValueError as error:
                                    errors.append(name + ": " + str(error))
                            if any(value not in previous for value in INPUTS):
                                errors.append(name + ": APT stage omits installer/parser/catalog COPY")
                    previous += raw
        except (ValueError, StopIteration) as error:
            errors.append(name + ": " + str(error))
    for name in ("scripts/install-release-dependencies.sh", ".github/scripts/install-linux-build-deps.sh"):
        text = (root / name).read_text()
        commands = "\n".join(line for line in text.splitlines() if not line.lstrip().startswith("#"))
        if RAW_APT.search(commands) or "apt-snapshot-install.sh" not in text or "security/apt-snapshots.tsv" not in text:
            errors.append(name + ": Linux dependencies must use reviewed APT snapshots")
    return errors
