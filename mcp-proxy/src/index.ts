import { randomUUID } from 'node:crypto';
import express from 'express';
import cors from 'cors';
import { StreamableHTTPServerTransport } from '@modelcontextprotocol/sdk/server/streamableHttp.js';
import type { Server } from '@modelcontextprotocol/sdk/server/index.js';
import { loadConfig } from './config.js';
import { NyxIdClient } from './nyxid-client.js';
import {
  createProtectedResourceMetadata,
  createBearerAuthMiddleware,
  getAuthContext,
} from './auth.js';
import { createMcpServer } from './server.js';

function isInitializeRequest(
  body: unknown,
): body is { method: 'initialize' } {
  return (
    typeof body === 'object' &&
    body !== null &&
    'method' in body &&
    (body as Record<string, unknown>).method === 'initialize'
  );
}

interface SessionData {
  readonly transport: StreamableHTTPServerTransport;
  readonly server: Server;
  readonly accessToken: string;
}

async function main() {
  const config = loadConfig();
  const nyxidClient = new NyxIdClient(config);

  // Discover NyxID OIDC configuration on startup
  const oidcConfig = await nyxidClient.discoverOidc();
  console.log(`OIDC discovery complete: issuer=${oidcConfig.issuer}`);

  const mcpResourceUrl = `http://localhost:${config.mcpPort}/mcp`;
  const resourceMetadataUrl = `http://localhost:${config.mcpPort}/.well-known/oauth-protected-resource`;

  const app = express();
  app.use(express.json());
  app.use(
    cors({
      exposedHeaders: [
        'WWW-Authenticate',
        'Mcp-Session-Id',
        'Mcp-Protocol-Version',
      ],
      origin: '*',
    }),
  );

  // OAuth 2.1 Protected Resource Metadata (RFC 9728)
  app.get('/.well-known/oauth-protected-resource', (_req, res) => {
    res.json(
      createProtectedResourceMetadata(mcpResourceUrl, oidcConfig.issuer),
    );
  });

  const authMiddleware = createBearerAuthMiddleware(
    nyxidClient,
    resourceMetadataUrl,
  );

  const sessions = new Map<string, SessionData>();

  // POST /mcp - Main MCP JSON-RPC endpoint
  app.post('/mcp', authMiddleware, async (req, res) => {
    const sessionId = req.headers['mcp-session-id'] as string | undefined;
    const { token } = getAuthContext(res);

    try {
      // Existing session: reuse transport
      if (sessionId && sessions.has(sessionId)) {
        const session = sessions.get(sessionId)!;
        await session.transport.handleRequest(req, res, req.body);
        return;
      }

      // New session: must be an initialize request
      if (!sessionId && isInitializeRequest(req.body)) {
        const mcpConfig = await nyxidClient.getMcpConfig(token);
        const server = createMcpServer(mcpConfig, token, nyxidClient);

        const transport = new StreamableHTTPServerTransport({
          sessionIdGenerator: () => randomUUID(),
          onsessioninitialized: (sid: string) => {
            sessions.set(sid, { transport, server, accessToken: token });
          },
        });

        transport.onclose = () => {
          const sid = transport.sessionId;
          if (sid) {
            sessions.delete(sid);
          }
        };

        await server.connect(transport);
        await transport.handleRequest(req, res, req.body);
        return;
      }

      res.status(400).json({
        jsonrpc: '2.0',
        error: {
          code: -32000,
          message: 'Bad Request: No valid session ID provided',
        },
        id: null,
      });
    } catch (error) {
      console.error('Error handling MCP POST:', error);
      if (!res.headersSent) {
        res.status(500).json({
          jsonrpc: '2.0',
          error: { code: -32603, message: 'Internal server error' },
          id: null,
        });
      }
    }
  });

  // GET /mcp - SSE stream for server-to-client notifications
  app.get('/mcp', authMiddleware, async (req, res) => {
    const sessionId = req.headers['mcp-session-id'] as string | undefined;

    if (!sessionId || !sessions.has(sessionId)) {
      res.status(400).json({
        jsonrpc: '2.0',
        error: {
          code: -32000,
          message: 'Invalid or missing session ID',
        },
        id: null,
      });
      return;
    }

    try {
      const session = sessions.get(sessionId)!;
      await session.transport.handleRequest(req, res);
    } catch (error) {
      console.error('Error handling MCP GET:', error);
      if (!res.headersSent) {
        res.status(500).json({
          jsonrpc: '2.0',
          error: { code: -32603, message: 'Internal server error' },
          id: null,
        });
      }
    }
  });

  // DELETE /mcp - Session termination
  app.delete('/mcp', authMiddleware, async (req, res) => {
    const sessionId = req.headers['mcp-session-id'] as string | undefined;

    if (!sessionId || !sessions.has(sessionId)) {
      res.status(400).json({
        jsonrpc: '2.0',
        error: {
          code: -32000,
          message: 'Invalid or missing session ID',
        },
        id: null,
      });
      return;
    }

    try {
      const session = sessions.get(sessionId)!;
      await session.transport.handleRequest(req, res);
    } catch (error) {
      console.error('Error handling MCP DELETE:', error);
      if (!res.headersSent) {
        res.status(500).json({
          jsonrpc: '2.0',
          error: { code: -32603, message: 'Internal server error' },
          id: null,
        });
      }
    }
  });

  // Health check
  app.get('/health', (_req, res) => {
    res.json({ status: 'ok', sessions: sessions.size });
  });

  app.listen(config.mcpPort, () => {
    console.log(`NyxID MCP Proxy listening on port ${config.mcpPort}`);
    console.log(`Protected Resource Metadata: ${resourceMetadataUrl}`);
  });

  // Graceful shutdown
  process.on('SIGINT', async () => {
    console.log('Shutting down...');
    for (const [sid, session] of sessions) {
      try {
        await session.transport.close();
      } catch (error) {
        console.error(`Error closing session ${sid}:`, error);
      }
    }
    sessions.clear();
    process.exit(0);
  });
}

main().catch((err) => {
  console.error('Failed to start MCP proxy:', err);
  process.exit(1);
});
