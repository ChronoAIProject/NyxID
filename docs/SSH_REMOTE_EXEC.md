# SSH Remote Command Execution

NyxID can execute commands on remote SSH services programmatically, via REST API or MCP tools. AI agents (Claude, GPT, or any MCP-compatible client) can use this capability to control remote machines through NyxID's authenticated proxy -- no direct SSH access or key distribution required.

Command execution uses the same SSH certificate infrastructure as tunneling. NyxID authenticates the caller, signs a short-lived certificate, opens a connection to the target, runs the command, and returns structured output.

---

## 1. REST API

### Endpoint

```
POST /api/v1/ssh/{service_id}/exec
```

### Authentication

- **Bearer token**: `Authorization: Bearer <access_token>`
- **Cookie session**: Automatically attached when calling from the NyxID frontend or CLI

The caller must have access to the SSH service. The service must have `certificate_auth_enabled: true` in its `ssh_config`.

### Request Body

| Field | Type | Required | Default | Description |
|-------|------|----------|---------|-------------|
| `command` | string | yes | -- | The shell command to execute on the remote host |
| `principal` | string | no | First entry in `allowed_principals` | Unix username to execute as (must be in the service's `allowed_principals`) |
| `timeout_secs` | integer | no | `30` | Maximum execution time in seconds (range: 1-300) |

```json
{
  "command": "ls -la /var/log",
  "principal": "ubuntu",
  "timeout_secs": 30
}
```

### Response

```json
{
  "exit_code": 0,
  "stdout": "total 1234\ndrwxr-xr-x  12 root root 4096 Mar 19 00:00 .\n...",
  "stderr": "",
  "duration_ms": 142,
  "timed_out": false
}
```

| Field | Type | Description |
|-------|------|-------------|
| `exit_code` | integer | Process exit code (0 = success) |
| `stdout` | string | Standard output (truncated at 1 MB) |
| `stderr` | string | Standard error (truncated at 1 MB) |
| `duration_ms` | integer | Wall-clock execution time in milliseconds |
| `timed_out` | boolean | `true` if the command was killed after `timeout_secs` elapsed |

### Error Responses

| Status | Code | Condition |
|--------|------|-----------|
| 400 | 1001 | Missing or invalid `command` field |
| 400 | 1002 | Command matches the blocklist |
| 400 | 1003 | `timeout_secs` out of range (1-300) |
| 403 | 2001 | Caller does not have access to the service |
| 403 | 2002 | Requested principal not in `allowed_principals` |
| 404 | 3001 | Service not found |
| 404 | 3002 | Service is not an SSH service or certificate auth is not enabled |
| 408 | 7000 | Command execution timed out (also returned in body with `timed_out: true`) |
| 502 | 8001 | Node is offline (for node-routed targets) |
| 504 | 8002 | Node proxy timeout (for node-routed targets) |

---

## 2. MCP Tools

NyxID exposes SSH command execution through its MCP transport at `/mcp`. Any MCP-compatible client (Claude Desktop, GPT agents, custom integrations) can call these tools after authenticating.

### `nyx__ssh_exec`

Execute a command on a remote SSH service.

**Parameters:**

| Parameter | Type | Required | Description |
|-----------|------|----------|-------------|
| `service` | string | yes | Service slug or UUID |
| `command` | string | yes | Shell command to execute |
| `principal` | string | no | Unix username (defaults to first allowed principal) |
| `timeout_secs` | integer | no | Max execution time, 1-300 (default: 30) |

**Returns:**

```json
{
  "stdout": "...",
  "stderr": "...",
  "exit_code": 0
}
```

If the command fails or times out, the tool returns an error with a descriptive message.

### `nyx__ssh_list_services`

List SSH services available to the authenticated user for command execution.

**Parameters:** None.

**Returns:** A list of SSH services with their IDs, slugs, names, and allowed principals.

### MCP Tool Call Example

```json
{
  "tool": "nyx__ssh_exec",
  "arguments": {
    "service": "prod-bastion",
    "command": "df -h",
    "principal": "ubuntu"
  }
}
```

---

## 3. Security

### Command Blocklist

NyxID rejects commands that match a server-side blocklist of destructive patterns. The following commands (and variations) are blocked:

- `rm -rf /`
- `mkfs`
- `dd if=` (targeting block devices)
- `shutdown`
- `reboot`
- `halt`
- `init 0` / `init 6`
- `:(){ :|:& };:` (fork bomb)
- `> /dev/sda` (direct device writes)
- `chmod -R 777 /`
- `chown -R` (on system directories)

The blocklist is applied as pattern matching against the raw command string before execution. It is a safety net, not a sandbox -- do not rely on it as the sole security boundary.

### Output Size Limit

Each of stdout and stderr is capped at **1 MB**. If a command produces more output, it is truncated and the response includes the first 1 MB. Commands that produce unbounded output (e.g., `cat /dev/urandom`) will be killed when the buffer fills or the timeout is reached.

### Execution Timeout

- **Default:** 30 seconds
- **Maximum:** 300 seconds (5 minutes)
- **Minimum:** 1 second

If a command exceeds its timeout, NyxID sends SIGKILL to the remote process, returns any output captured so far, and sets `timed_out: true` in the response.

### Audit Logging

Every command execution is logged as an `ssh_command_executed` audit event with:

- User ID and IP address
- Service ID and service name
- Command string (full text)
- Principal used
- Exit code and whether it timed out
- Execution duration

Blocked commands are logged as `ssh_command_blocked` events before rejection.

### Certificate-Based Authentication

Command execution uses the same short-lived SSH certificate infrastructure as tunneling. No passwords are stored or transmitted. The certificate is issued for the requested principal and is valid only for the duration of the execution.

### Access Control

- Standard NyxID RBAC applies: the caller must have access to the SSH service
- Per-service approval configs can require human approval before commands execute (via the approvals system)
- The principal must be listed in the service's `allowed_principals`
- Admin users can execute on any SSH service

---

## 4. Node Routing

Commands can target SSH services behind firewalls or private networks using NyxID's node agent infrastructure. The routing is transparent to the caller.

### How It Works

1. Caller sends `POST /api/v1/ssh/{service_id}/exec`
2. NyxID resolves the node route for the service (same logic as SSH tunneling)
3. If a healthy node binding exists, NyxID forwards the command execution request to the node agent
4. The node agent opens a local connection to the SSH target, executes the command, and returns the result
5. If no node route is available, NyxID connects directly to the SSH target

### Requirements

- A node agent must be deployed on a machine that can reach the SSH target
- The node must be registered with NyxID and bound to the SSH service
- The SSH target must be in the node's `ssh.allowed_targets` list (for private/loopback addresses)

See [SSH_TUNNELING.md](SSH_TUNNELING.md) section 6 for full node agent setup instructions.

---

## 5. Use Cases

### AI Agent Automation

AI agents (Claude, GPT, or custom LLM agents) can manage remote infrastructure through NyxID's MCP tools. The agent authenticates once with NyxID and can then execute commands on any SSH service the user has access to -- no SSH key distribution or direct network access required.

### CI/CD Pipeline Integration

CI/CD pipelines can use the REST API to execute deployment commands, run database migrations, or validate infrastructure state on remote servers. Use an API key for authentication in automated environments.

### Remote Server Monitoring and Health Checks

Periodically execute diagnostic commands (`df -h`, `free -m`, `systemctl status`, `docker ps`) on remote servers to collect health metrics without maintaining persistent connections.

### Automated Deployment Scripts

Chain multiple exec calls to perform multi-step deployments: pull code, run migrations, restart services, and verify health -- all through authenticated API calls.

### Infrastructure Management

Execute administrative commands across multiple SSH services: rotate logs, update packages, check certificate expiry, or collect inventory information from a fleet of servers.

---

## 6. Examples

### curl: Execute a Command

```bash
curl -X POST https://auth.example.com/api/v1/ssh/SERVICE_ID/exec \
  -H "Authorization: Bearer $NYXID_ACCESS_TOKEN" \
  -H "Content-Type: application/json" \
  -d '{
    "command": "uptime",
    "principal": "ubuntu"
  }'
```

Response:

```json
{
  "exit_code": 0,
  "stdout": " 14:23:01 up 42 days,  3:15,  0 users,  load average: 0.12, 0.08, 0.05\n",
  "stderr": "",
  "duration_ms": 87,
  "timed_out": false
}
```

### curl: Execute with Custom Timeout

```bash
curl -X POST https://auth.example.com/api/v1/ssh/SERVICE_ID/exec \
  -H "Authorization: Bearer $NYXID_ACCESS_TOKEN" \
  -H "Content-Type: application/json" \
  -d '{
    "command": "apt list --upgradable 2>/dev/null",
    "principal": "ubuntu",
    "timeout_secs": 60
  }'
```

### curl: Check Disk Usage

```bash
curl -X POST https://auth.example.com/api/v1/ssh/SERVICE_ID/exec \
  -H "Authorization: Bearer $NYXID_ACCESS_TOKEN" \
  -H "Content-Type: application/json" \
  -d '{
    "command": "df -h --output=target,pcent,avail | head -20",
    "principal": "ubuntu"
  }'
```

### MCP: Execute via Claude or GPT Agent

When an AI agent is connected to NyxID's MCP server, it can call tools directly:

```json
{
  "tool": "nyx__ssh_exec",
  "arguments": {
    "service": "prod-bastion",
    "command": "docker ps --format 'table {{.Names}}\t{{.Status}}\t{{.Ports}}'",
    "principal": "deploy",
    "timeout_secs": 15
  }
}
```

### MCP: List Available SSH Services

```json
{
  "tool": "nyx__ssh_list_services",
  "arguments": {}
}
```

### MCP: Multi-Step Deployment

An AI agent can chain calls to perform a deployment:

```
1. nyx__ssh_exec(service="prod-api", command="cd /app && git pull origin main", principal="deploy")
2. nyx__ssh_exec(service="prod-api", command="cd /app && cargo build --release", principal="deploy", timeout_secs=300)
3. nyx__ssh_exec(service="prod-api", command="sudo systemctl restart myapp", principal="deploy")
4. nyx__ssh_exec(service="prod-api", command="curl -sf http://localhost:8080/health", principal="deploy")
```

### Using the CLI

The `nyxid` CLI can also execute remote commands:

```bash
nyxid ssh exec \
  --base-url https://auth.example.com \
  --service-id SERVICE_ID \
  --principal ubuntu \
  --command "systemctl status nginx"
```

With a timeout:

```bash
nyxid ssh exec \
  --base-url https://auth.example.com \
  --service-id SERVICE_ID \
  --principal deploy \
  --timeout 120 \
  --command "cd /app && npm run migrate"
```
