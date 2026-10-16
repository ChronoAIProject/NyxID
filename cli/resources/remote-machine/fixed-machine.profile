# NyxID Remote Machine MVP fixed profile: remote-machine
#
# Full hosted Machine policy record. managed-start applies sealed owner choices;
# the node-only setup preset is a separate limited shell/files build helper.
profile_version = "remote-machine"
default_capabilities = ["shell", "files"]
supported_capabilities = ["shell", "files", "browser", "computer"]
# Computer requires Browser; local enablement never grants caller authority.
workspace_root = "/workspace"
computer = false
browser = false
computer_mode = "standard"
max_jobs = 2
max_timeout_secs = 3600
per_task_output_bytes = 1048576
whole_machine_output_bytes = 33554432
command_bytes = 32768
stdin_bytes = 65536
workspace_size = "20Gi"
# Temporary storage is deployment-managed; no fixed allocation is declared.
cpu = "2"
memory = "2Gi"
network = "outbound-node-wss-and-gateway"
service = false

# The image digest, optional RuntimeClass and optional existing StorageClass
# selection are deployment-owned; omitted storage uses the cluster default.
# Agent Sandbox owns the native lifecycle; independent
# physical stop/writer fencing remains deferred. Admission requires a pinned image.
