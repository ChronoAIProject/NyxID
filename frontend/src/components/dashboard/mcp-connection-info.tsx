import { useUiTranslation } from "@/i18n/use-ui-translation";
import { translateUi } from "@/i18n/use-ui-translation";
import { translateErrorMessage } from "@/lib/error-messages";
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

function buildCodexConfig(mcpUrl: string): string {
  return `[mcp_servers.nyxid]\nurl = "${mcpUrl}"`;
}

function CopyButton({
  text,
  label,
}: {
  readonly text: string;
  readonly label: string;
}) {
  useUiTranslation();
  const [copied, setCopied] = useState(false);

  async function handleCopy() {
    try {
      await copyToClipboard(text);
      setCopied(true);
      toast.success(translateUi(`${label} copied to clipboard`));
      setTimeout(() => setCopied(false), 2000);
    } catch {
      toast.error(translateErrorMessage("Failed to copy to clipboard"));
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
        <Check className="h-3 w-3 text-success" />
      ) : (
        <Copy className="h-3 w-3" />
      )}
      <span className="sr-only">{translateUi("Copy ")}{label}</span>
    </Button>
  );
}

export function McpConnectionInfo() {
  useUiTranslation();
  const { data: config, isLoading } = usePublicConfig();

  if (isLoading) {
    return <Skeleton className="h-64 w-full" />;
  }

  const mcpUrl = config?.mcp_url ?? `${window.location.origin}/mcp`;
  const cursorConfig = buildCursorConfig(mcpUrl);
  const claudeCodeConfig = buildClaudeCodeConfig(mcpUrl);
  const codexConfig = buildCodexConfig(mcpUrl);

  return (
    <div className="space-y-4">
      <div className="rounded-lg border border-border/50 bg-muted/30 p-3">
        <p className="text-13 text-muted-foreground">{translateUi("NyxID exposes a single MCP endpoint that provides tools for all your connected services. Connect your MCP client once and it will automatically discover tools from every service you have enabled.")}</p>
      </div>

      <div>
        <p className="mb-1 type-secondary-emphasis text-muted-foreground">{translateUi("MCP Proxy URL")}</p>
        <div className="relative">
          <code className="block rounded bg-muted px-3 py-2 pr-10 text-14 break-all">
            {mcpUrl}
          </code>
          <CopyButton text={mcpUrl} label={translateUi("MCP proxy URL")} />
        </div>
      </div>

      <div>
        <div className="mb-1 flex items-center gap-2">
          <p className="type-secondary-emphasis text-muted-foreground">{translateUi("Cursor Configuration")}</p>
          <Badge variant="secondary" className="text-12">
            .cursor/mcp.json
          </Badge>
        </div>
        <div className="relative">
          <pre className="whitespace-pre-wrap break-all rounded-lg bg-muted px-4 py-3.5 pr-10 min-h-[44px] text-14">
            {cursorConfig}
          </pre>
          <CopyButton text={cursorConfig} label={translateUi("Cursor config")} />
        </div>
      </div>

      <div>
        <div className="mb-1 flex items-center gap-2">
          <p className="type-secondary-emphasis text-muted-foreground">{translateUi("Claude Code Configuration")}</p>
          <Badge variant="secondary" className="text-12">
            .claude/settings.json
          </Badge>
        </div>
        <div className="relative">
          <pre className="whitespace-pre-wrap break-all rounded-lg bg-muted px-4 py-3.5 pr-10 min-h-[44px] text-14">
            {claudeCodeConfig}
          </pre>
          <CopyButton text={claudeCodeConfig} label={translateUi("Claude Code config")} />
        </div>
      </div>

      <div>
        <div className="mb-1 flex items-center gap-2">
          <p className="type-secondary-emphasis text-muted-foreground">{translateUi("Codex Configuration")}</p>
          <Badge variant="secondary" className="text-12">
            ~/.codex/config.toml
          </Badge>
        </div>
        <div className="relative">
          <pre className="whitespace-pre-wrap break-all rounded-lg bg-muted px-4 py-3.5 pr-10 min-h-[44px] text-14">
            {codexConfig}
          </pre>
          <CopyButton text={codexConfig} label={translateUi("Codex config")} />
        </div>
      </div>

      <div className="rounded-lg border border-border/50 bg-muted/30 p-3">
        <p className="type-secondary-emphasis mb-1">{translateUi("How it works")}</p>
        <p className="text-13 text-muted-foreground">{translateUi("When an MCP client connects, NyxID authenticates via OAuth in your browser. Once authenticated, the proxy exposes tools from all your connected services. Tool calls are forwarded to each service's API with your credentials injected automatically.")}</p>
      </div>
    </div>
  );
}
