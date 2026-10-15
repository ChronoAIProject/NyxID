#!/usr/bin/env python3
"""Render selected read-only observer RBAC; never contact or mutate a cluster."""
import argparse
import json
import re

parser = argparse.ArgumentParser()
parser.add_argument("--controller-namespace", required=True)
parser.add_argument("--controller-deployment", required=True)
parser.add_argument("--telemetry-node", action="append", default=[])
args = parser.parse_args()


def valid(value, dotted=False):
    return len(value) <= 253 and all(
        re.fullmatch(r"[a-z0-9](?:[a-z0-9-]{0,61}[a-z0-9])?", label)
        for label in (value.split(".") if dotted else [value])
    )


if not valid(args.controller_namespace) or not valid(args.controller_deployment, True):
    parser.error("selected controller namespace/deployment must be DNS names")
nodes = sorted(set(args.telemetry_node))
if len(nodes) > 256 or any(not valid(node, True) for node in nodes):
    parser.error("telemetry nodes must be at most 256 distinct DNS names")
subject = {"kind": "ServiceAccount", "name": "nyxid-machine-manager", "namespace": "nyxid-machine"}
role = {"apiVersion": "rbac.authorization.k8s.io/v1", "kind": "Role",
        "metadata": {"name": "nyxid-machine-controller-observer", "namespace": args.controller_namespace},
        "rules": [{"apiGroups": ["apps"], "resources": ["deployments"],
                   "resourceNames": [args.controller_deployment], "verbs": ["get"]}]}
binding = {"apiVersion": "rbac.authorization.k8s.io/v1", "kind": "RoleBinding",
           "metadata": role["metadata"], "subjects": [subject],
           "roleRef": {"apiGroup": "rbac.authorization.k8s.io", "kind": "Role", "name": role["metadata"]["name"]}}
items = [role, binding]
if nodes:
    name = "nyxid-machine-selected-node-stats"
    items += [{"apiVersion": "rbac.authorization.k8s.io/v1", "kind": "ClusterRole", "metadata": {"name": name},
               "rules": [{"apiGroups": [""], "resources": ["nodes/stats"], "resourceNames": nodes, "verbs": ["get"]}]},
              {"apiVersion": "rbac.authorization.k8s.io/v1", "kind": "ClusterRoleBinding", "metadata": {"name": name},
               "subjects": [subject], "roleRef": {"apiGroup": "rbac.authorization.k8s.io", "kind": "ClusterRole", "name": name}}]
print(json.dumps({"apiVersion": "v1", "kind": "List", "items": items}, indent=2))
