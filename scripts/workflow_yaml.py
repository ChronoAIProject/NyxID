"""Strict YAML parsing for GitHub workflow contracts (including nested maps)."""
import yaml


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
                if step.get("with", {}).get("toolchain") != "1.98.1":
                    errors.append(f"{job_name} step {index}: pinned Rust Action must select toolchain 1.98.1 explicitly")
    return errors
