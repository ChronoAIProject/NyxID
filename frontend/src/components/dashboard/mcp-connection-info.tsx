import { useState } from "react";
import { usePublicConfig } from "@/hooks/use-public-config";
import { copyToClipboard } from "@/lib/utils";
import { Button } from "@/components/ui/button";
import { Badge } from "@/components/ui/badge";
import { Skeleton } from "@/components/ui/skeleton";
import { Check, Copy } from "lucide-react";
import { toast } from "sonner";

function buildCursorConfig(mcpUrl: string): string {
  return JSON.stringify(
    {
      mcpServers: {
        nyxid: {
          url: mcpUrl,
          description: "NyxID MCP Proxy",
        },
      },
    },
    null,
    2,
  );
}

function buildClaudeCodeConfig(mcpUrl: string): string {
  return JSON.stringify(
    {
      mcpServers: {
        nyxid: {
          command: "npx",
          args: ["-y", "@anthropic-ai/mcp-proxy", mcpUrl],
          description: "NyxID MCP Proxy",
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

export function McpConnectionInfo() {
  const { data: config, isLoading } = usePublicConfig();

  if (isLoading) {
    return <Skeleton className="h-64 w-full" />;
  }

  const mcpUrl = config?.mcp_url ?? `${window.location.origin}/mcp`;
  const cursorConfig = buildCursorConfig(mcpUrl);
  const claudeCodeConfig = buildClaudeCodeConfig(mcpUrl);

  return (
    <div className="space-y-4">
      <div className="rounded-md border border-border/50 bg-muted/30 p-3">
        <p className="text-xs text-muted-foreground">
          NyxID exposes a single MCP endpoint that provides tools for all your
          connected services. Connect your MCP client once and it will
          automatically discover tools from every service you have enabled.
        </p>
      </div>

      <div>
        <p className="mb-1 text-xs font-medium text-muted-foreground">
          MCP Proxy URL
        </p>
        <div className="relative">
          <code className="block rounded bg-muted px-3 py-2 pr-10 text-xs break-all">
            {mcpUrl}
          </code>
          <CopyButton text={mcpUrl} label="MCP proxy URL" />
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
          When an MCP client connects, NyxID authenticates via OAuth in your
          browser. Once authenticated, the proxy exposes tools from all your
          connected services. Tool calls are forwarded to each service's API
          with your credentials injected automatically.
        </p>
      </div>
    </div>
  );
}
