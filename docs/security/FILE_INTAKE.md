# File intake inventory

This inventory covers the file intake paths in the NyxID application. Match
enabled channel adapters, catalog operations, and deployment-specific limits
to the assessed release.

| Surface | Accepted input and limits | Storage and retrieval |
|---|---|---|
| Assistant conversations and group threads | Browser uploads, including drag and drop and paste, use the conversation or group attachment route. NyxID checks PNG, JPEG, GIF, WebP, PDF, DOCX, and UTF-8 plain text, Markdown, CSV, and JSON. Limits are 20 MiB per file, ten per message, and 30 upload attempts per owner per minute. | Authorization binds each upload to its owner and conversation or group. NyxID envelope-encrypts bytes and extraction results. Downloads use owner or group authorization and the attachment response protections. |
| Channel messages and media | Enabled channel adapters provide image, document, or audio inputs according to the provider contract. The relay bounds media fetches and accepts bounded encoded data on supported reply paths. Assistant imports apply the separate assistant attachment checks. | The relay retains provider references and metadata. Attachment retrieval verifies the message, bot, and owner authority. Assistant imports use encrypted attachment storage. |
| Catalog and Tool icons | Catalog and Tool icon upload routes accept SVG in JSON. The routes cap the body at 256 KiB. The SVG validator enforces its own size, element, attribute, and reference rules. | NyxID stores a validated asset and serves it with a fixed SVG content type and sandbox and content-sniffing restrictions. |
| Machine file operations | Authorized machine tools accept bounded text or base64 bytes. The machine enforces path roots, operation limits, and file-write authority. | Files stay in the selected machine context. An explicit share can copy an eligible file into the assistant attachment system. |
| Proxy and catalog operation uploads | HTTP and MCP execution accept request bodies according to the configured operation and transport limits. Eligible assistant calls can pass an authorized attachment to a published binary operation. Managed connector uploads apply the connector's own policy. | NyxID authorizes the operation and forwards the request to the selected destination. File storage at that destination follows its contract. |
| Built-in Oracle Relay | Consumer and worker routes under `/api/v1/oracle` return `410 Gone` before the handler runs. This includes task attachments, worker result uploads, and login snapshot or profile intake. | The historical contract in the Oracle Relay design describes the former PDF and generic attachment fields and the image and file result arrays. |

Avatar and organization icon URL fields accept URL metadata. Direct catalog and
Tool icon uploads use the validated SVG paths above.

[Assistant uploads](../chat/10-uploads.md) ·
[Channel relay media](../CHANNEL_BOT_RELAY.md) ·
[Machine file tools](../MACHINE_NODES.md) ·
[Catalog operation contracts](../API_DISCOVERY.md) ·
[Oracle Relay retirement and historical design](../ORACLE_RELAY.md)
