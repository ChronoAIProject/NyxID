import { Server } from '@modelcontextprotocol/sdk/server/index.js';
import {
  ListToolsRequestSchema,
  CallToolRequestSchema,
} from '@modelcontextprotocol/sdk/types.js';
import type { McpConfig } from './types.js';
import type { NyxIdClient } from './nyxid-client.js';
import {
  generateToolDefinitions,
  resolveToolCall,
  buildProxyArgs,
} from './tools.js';

export function createMcpServer(
  mcpConfig: McpConfig,
  accessToken: string,
  nyxidClient: NyxIdClient,
): Server {
  const server = new Server(
    { name: 'nyxid-mcp-proxy', version: '0.1.0' },
    { capabilities: { tools: {} } },
  );

  const tools = generateToolDefinitions(mcpConfig);

  server.setRequestHandler(ListToolsRequestSchema, async () => ({
    tools: tools.map((t) => ({
      name: t.name,
      description: t.description,
      inputSchema: t.inputSchema,
    })),
  }));

  server.setRequestHandler(CallToolRequestSchema, async (request) => {
    const { name, arguments: args } = request.params;

    const resolved = resolveToolCall(name, mcpConfig);
    if (!resolved) {
      return {
        content: [{ type: 'text' as const, text: `Unknown tool: ${name}` }],
        isError: true,
      };
    }

    const { service, endpoint } = resolved;
    const proxyArgs = buildProxyArgs(
      endpoint,
      (args ?? {}) as Record<string, unknown>,
    );

    try {
      const result = await nyxidClient.proxyRequest(
        accessToken,
        service.id,
        proxyArgs.method,
        proxyArgs.path,
        Object.keys(proxyArgs.query).length > 0
          ? proxyArgs.query
          : undefined,
        proxyArgs.body,
      );

      const responseText =
        typeof result.body === 'string'
          ? result.body
          : JSON.stringify(result.body, null, 2);

      return {
        content: [
          {
            type: 'text' as const,
            text:
              result.status >= 400
                ? `Error (${result.status}): ${responseText}`
                : responseText,
          },
        ],
        isError: result.status >= 400,
      };
    } catch (error) {
      const message =
        error instanceof Error ? error.message : String(error);
      return {
        content: [
          { type: 'text' as const, text: `Proxy error: ${message}` },
        ],
        isError: true,
      };
    }
  });

  return server;
}
