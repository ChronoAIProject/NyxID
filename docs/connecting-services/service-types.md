# Service types and connections

NyxID uses several fields to describe a service. They answer different questions: what users can connect, how NyxID reaches it, and where its credential lives. MQTT is an HTTP service in these fields because NyxID calls a local HTTP bridge; the bridge speaks MQTT to the broker.

## Catalog entries and connections

| Term | Meaning | Example |
|---|---|---|
| Catalog entry | A shared `DownstreamService` template with a name, default endpoint, authentication metadata, and optional API operations. | `api-mqtt` describes the MQTT bridge. |
| Provider | A `ProviderConfig` that describes a credential or authorization method. A provider can back more than one catalog entry. | `mqtt` describes bridge-key setup for `api-mqtt`. |
| Connection | A `UserService` owned by a person or organization, with its own slug, endpoint, credential binding, and route. | A connection created from `api-mqtt` can be named `api-mqtt-2`. |
| Agent Key | A NyxID key that authorizes an agent to call permitted connections. It is separate from the downstream credential. | A scoped `nyx_...` key can call an MQTT connection. |

Use the returned connection slug for proxy requests and node credential setup. Use the catalog slug when you search or add a catalog entry.

## Transport type

`service_type` describes the transport NyxID manages:

| Value | NyxID behavior | Examples |
|---|---|---|
| `http` | Sends HTTP requests through its proxy. An OpenAPI document can describe operations for discovery. | REST APIs, IFTTT, and the MQTT HTTP bridge. |
| `ssh` | Opens an SSH service through a credential node using the configured SSH authentication mode. | An internal host reached with `nyxid ssh`. |

NyxID has no native `mqtt` `service_type`. The MQTT broker connection, subscriptions, and topic mapping belong to the [local bridge](mqtt.md).

## Service category

`service_category` describes how a catalog service is offered:

| Value | Meaning |
|---|---|
| `connection` | A person or organization creates a connection. It can use its own credential, a node-held credential, or an allowed platform key. `api-mqtt` and `api-ifttt` use this category. |
| `internal` | An administrator manages the service or its shared credential. Access can depend on service policy and platform-key grants. |
| `provider` | An OIDC identity-provider service managed by an administrator. It is not a downstream proxy connection. |

The provider *record* and the `provider` *category* are different concepts. A `connection` service can link to a provider record without becoming a `provider` category service.

## Authentication and route

Authentication describes how the downstream service accepts a credential. Catalog entries can use a header or bearer key, OAuth, device-code login, or another supported method. Some services need no user-supplied credential, and some offer an authorized platform key. These choices do not change `service_type`.

`ProviderConfig.provider_type` describes the provider's credential flow. `DownstreamService.auth_method` describes how NyxID sends the credential to the HTTP endpoint. For MQTT, `provider_type` is `api_key` because setup takes a bridge key, while `auth_method` is `bearer` because the bridge expects an `Authorization: Bearer` header. `requires_gateway_url` means each connection must supply its bridge URL when using direct routing; a node can store the target URL locally.

The route describes where the downstream request runs:

| Route | Request path | Credential location |
|---|---|---|
| Direct | NyxID server to downstream HTTP endpoint. In hosted mode, user-supplied URLs must resolve to public addresses. | NyxID stores user-supplied credentials encrypted, or uses an authorized platform credential. |
| Via Node | NyxID forwards to the selected credential node, which calls the downstream endpoint. The node connects outbound to NyxID. | A node-managed credential stays in the node's local encrypted store. |

For local MQTT, choose **Via Node**. The bridge listens only on loopback, and its broker credentials stay in its environment. See [Connect MQTT devices](mqtt.md) for the full procedure. For general node setup, see [nyxid node Agent](../NYXID_NODE.md).

## Operations and approvals

A catalog OpenAPI spec gives NyxID named operations and schemas for discovery. It does not create a connection or grant an agent access. Tool markers such as `readOnly`, `destructive`, and `requiresApproval` describe an operation to clients and admission systems. Configure NyxID runtime approval policy separately when a write needs human review. See [API Discovery and Catalog](../API_DISCOVERY.md) for spec discovery and [Connecting AI Services](README.md) for connection methods.
