"""Strict YAML parsing for GitHub workflow contracts (including nested maps)."""
import yaml

RUST_ACTION = "dtolnay/rust-toolchain@18f0487477426dce244608110aa2660c2893deb7"
# This version-specific Action hardcodes 1.98.1 in its parse step. Its only
# inputs are targets, target and components; `toolchain` is not an input.
RUST_ACTION_INPUTS = {"targets", "target", "components"}
RUST_COMPONENTS = {"rustfmt", "clippy"}


class UniqueKeyLoader(yaml.SafeLoader):
    pass


def unique_mapping(loader, node, deep=False):
    loader.flatten_mapping(node)
    mapping = {}
    for key_node, value_node in node.value:
        key = loader.construct_object(key_node, deep=deep)
        if key in mapping:
            raise yaml.constructor.ConstructorError("while constructing a mapping", node.start_mark,
                                                    "duplicate mapping key: " + str(key), key_node.start_mark)
        mapping[key] = loader.construct_object(value_node, deep=deep)
    return mapping


UniqueKeyLoader.add_constructor(yaml.resolver.BaseResolver.DEFAULT_MAPPING_TAG, unique_mapping)


def load_unique(text):
    return yaml.load(text, Loader=UniqueKeyLoader)


def validate_rust_steps(workflow):
    errors = []
    for job_name, job in workflow.get("jobs", {}).items():
        for index, step in enumerate(job.get("steps", [])):
            if step.get("uses", "").startswith("dtolnay/rust-toolchain@"):
                where = f"{job_name} step {index}"
                if step["uses"] != RUST_ACTION:
                    errors.append(where + ": Rust Action must pin the reviewed 1.98.1 implementation")
                inputs = step.get("with", {})
                if set(inputs) - RUST_ACTION_INPUTS:
                    errors.append(where + ": unsupported input for the version-specific Rust Action")
                components = inputs.get("components", "")
                if not isinstance(components, str) or not RUST_COMPONENTS.issubset(
                        {part.strip() for part in components.split(",")}):
                    errors.append(where + ": install repository rustfmt and clippy requirements during setup")
    return errors
