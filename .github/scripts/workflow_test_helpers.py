"""Read annotated action versions alongside the executable references in workflow tests."""
import re
from pathlib import Path

import yaml

from ci_gate_reuse import strict_load_workflow


USES_WITH_TAG = object()


def load_workflow_with_action_tags(path):
    source = Path(path).read_text()
    workflow = strict_load_workflow(source)
    lines = source.splitlines()

    def add_tags(node, parsed):
        if isinstance(node, yaml.SequenceNode):
            for item, value in zip(node.value, parsed):
                add_tags(item, value)
        elif isinstance(node, yaml.MappingNode):
            for key, value in node.value:
                if key.value != "uses" or not isinstance(value, yaml.ScalarNode):
                    add_tags(value, parsed[key.value])
                    continue
                pin = re.fullmatch(r"([^@\s]+)@[0-9a-f]{40}", value.value)
                if pin and value.start_mark.line == value.end_mark.line:
                    end = value.end_mark
                    comment = lines[end.line][end.column:]
                    tag = re.fullmatch(r"[ \t]+#[ \t]*([^#\s]+)[ \t]*", comment)
                    if tag:
                        parsed[USES_WITH_TAG] = f"{pin[1]}@{tag[1]}"

    # YAML discards comments when constructing a mapping. The scalar's source
    # position identifies the annotation for this particular use of the action.
    add_tags(yaml.compose(source, Loader=yaml.BaseLoader), workflow)
    return workflow
