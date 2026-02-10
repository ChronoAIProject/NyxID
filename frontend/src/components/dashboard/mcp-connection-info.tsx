import { useState } from "react";
import { copyToClipboard } from "@/lib/utils";
import { Button } from "@/components/ui/button";
import { Badge } from "@/components/ui/badge";
import { Check, Copy } from "lucide-react";
import { toast } from "sonner";

interface McpConnectionInfoProps {
  readonly serviceSlug: string;
  readonly serviceName: string;
}

function buildMcpProxyUrl(serviceSlug: string): string {
  const origin = typeof window !== "undefined" ? window.location.origin : "";
  return `${origin}/api/v1/mcp/${serviceSlug}`;
}

function buildCursorConfig(
  serviceSlug: string,
  serviceName: string,
): string {
  const url = buildMcpProxyUrl(serviceSlug);
  return JSON.stringify(
    {
      mcpServers: {
        [serviceSlug]: {
          url,
          description: serviceName,
        },
      },
    },
    null,
    2,
  );
}

function buildClaudeCodeConfig(
  serviceSlug: string,
  serviceName: string,
): string {
  const url = buildMcpProxyUrl(serviceSlug);
  return JSON.stringify(
    {
      mcpServers: {
        [serviceSlug]: {
          command: "npx",
          args: ["-y", "@anthropic-ai/mcp-proxy", url],
          description: serviceName,
        },
      },
    },
    null,
    2,
  );
}

function CopyButton({ text, label }: { readonly text: string; readonly label: string }) {
  const [copied, setCopied] = useState(false);

  async function handleCopy() {
    try {
      await copyToClipboard(text);
      setCopied(true);
      toast.success(`${label} copied to clipboard`);
      setTimeout(() => setCopied(false), 2000);
    } catch {
      toast.error("Failed to copy to clipboard");
    }
  }

  return (
    <Button
      variant="ghost"
      size="icon"
      className="absolute right-2 top-2 h-6 w-6"
      onClick={() => void handleCopy()}
    >
      {copied ? (
        <Check className="h-3 w-3 text-green-400" />
      ) : (
        <Copy className="h-3 w-3" />
      )}
      <span className="sr-only">Copy {label}</span>
    </Button>
  );
}

export function McpConnectionInfo({
  serviceSlug,
  serviceName,
}: McpConnectionInfoProps) {
  const proxyUrl = buildMcpProxyUrl(serviceSlug);
  const cursorConfig = buildCursorConfig(serviceSlug, serviceName);
  const claudeCodeConfig = buildClaudeCodeConfig(serviceSlug, serviceName);

  return (
    <div className="space-y-4">
      <div>
        <p className="mb-1 text-xs font-medium text-muted-foreground">
          MCP Proxy URL
        </p>
        <div className="relative">
          <code className="block rounded bg-muted px-3 py-2 pr-10 text-xs break-all">
            {proxyUrl}
          </code>
          <CopyButton text={proxyUrl} label="MCP proxy URL" />
        </div>
      </div>

      <div>
        <div className="mb-1 flex items-center gap-2">
          <p className="text-xs font-medium text-muted-foreground">
            Cursor Configuration
          </p>
          <Badge variant="outline" className="text-[10px]">
            .cursor/mcp.json
          </Badge>
        </div>
        <div className="relative">
          <pre className="rounded bg-muted px-3 py-2 pr-10 text-xs overflow-x-auto">
            {cursorConfig}
          </pre>
          <CopyButton text={cursorConfig} label="Cursor config" />
        </div>
      </div>

      <div>
        <div className="mb-1 flex items-center gap-2">
          <p className="text-xs font-medium text-muted-foreground">
            Claude Code Configuration
          </p>
          <Badge variant="outline" className="text-[10px]">
            .claude/settings.json
          </Badge>
        </div>
        <div className="relative">
          <pre className="rounded bg-muted px-3 py-2 pr-10 text-xs overflow-x-auto">
            {claudeCodeConfig}
          </pre>
          <CopyButton text={claudeCodeConfig} label="Claude Code config" />
        </div>
      </div>

      <div className="rounded-md border border-border/50 bg-muted/30 p-3">
        <p className="text-xs font-medium mb-1">How it works</p>
        <p className="text-xs text-muted-foreground">
          NyxID acts as an MCP proxy that authenticates requests via OAuth.
          When an MCP client connects, NyxID handles the OAuth authorization
          flow with the downstream service and proxies tool calls to the
          service's API endpoints on behalf of the authenticated user.
        </p>
      </div>
    </div>
  );
}
