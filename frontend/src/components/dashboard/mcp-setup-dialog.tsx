import { useState } from "react";
import { usePublicConfig } from "@/hooks/use-public-config";
import { copyToClipboard } from "@/lib/utils";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { Button } from "@/components/ui/button";
import { Badge } from "@/components/ui/badge";
import { Check, Copy, CheckCircle2 } from "lucide-react";
import { toast } from "sonner";

interface McpSetupDialogProps {
  readonly providerName: string;
  readonly open: boolean;
  readonly onClose: () => void;
}

type Tab = "claude-code" | "cursor" | "codex";

const TABS: readonly { readonly id: Tab; readonly label: string; readonly file: string }[] = [
  { id: "claude-code", label: "Claude Code", file: "~/.claude/settings.json" },
  { id: "cursor", label: "Cursor", file: ".cursor/mcp.json" },
  { id: "codex", label: "Codex", file: "~/.codex/config.toml" },
];

function buildConfig(tab: Tab, mcpUrl: string): string {
  switch (tab) {
    case "cursor":
      return JSON.stringify(
        { mcpServers: { nyxid: { url: mcpUrl } } },
        null,
        2,
      );
    case "claude-code":
      return JSON.stringify(
        {
          mcpServers: {
            nyxid: {
              command: "npx",
              args: ["-y", "@anthropic-ai/mcp-proxy", mcpUrl],
            },
          },
        },
        null,
        2,
      );
    case "codex":
      return `[mcp_servers.nyxid]\nurl = "${mcpUrl}"`;
  }
}

export function McpSetupDialog({
  providerName,
  open,
  onClose,
}: McpSetupDialogProps) {
  const { data: config } = usePublicConfig();
  const [activeTab, setActiveTab] = useState<Tab>("claude-code");
  const [copied, setCopied] = useState(false);

  const mcpUrl = config?.mcp_url ?? `${window.location.origin}/mcp`;
  const configSnippet = buildConfig(activeTab, mcpUrl);
  const activeFile = TABS.find((t) => t.id === activeTab)?.file ?? "";

  async function handleCopy() {
    try {
      await copyToClipboard(configSnippet);
      setCopied(true);
      toast.success("Config copied to clipboard");
      setTimeout(() => setCopied(false), 2000);
    } catch {
      toast.error("Failed to copy");
    }
  }

  return (
    <Dialog open={open} onOpenChange={(v) => !v && onClose()}>
      <DialogContent className="sm:max-w-lg">
        <DialogHeader>
          <DialogTitle className="flex items-center gap-2">
            <CheckCircle2 className="h-5 w-5 text-green-500" />
            Connected to {providerName}
          </DialogTitle>
          <DialogDescription>
            Your API is now available as MCP tools. Copy the config below into
            your AI tool to start using it.
          </DialogDescription>
        </DialogHeader>

        <div className="space-y-4">
          <div className="flex gap-1">
            {TABS.map((tab) => (
              <Button
                key={tab.id}
                variant={activeTab === tab.id ? "default" : "outline"}
                size="sm"
                onClick={() => setActiveTab(tab.id)}
              >
                {tab.label}
              </Button>
            ))}
          </div>

          <div>
            <div className="mb-1 flex items-center gap-2">
              <Badge variant="outline" className="text-[10px]">
                {activeFile}
              </Badge>
            </div>
            <div className="relative">
              <pre className="max-h-48 overflow-auto rounded bg-muted px-3 py-2 pr-10 text-xs">
                {configSnippet}
              </pre>
              <Button
                variant="ghost"
                size="icon"
                className="absolute right-2 top-2 h-6 w-6"
                onClick={() => void handleCopy()}
              >
                {copied ? (
                  <Check className="h-3 w-3 text-green-500" />
                ) : (
                  <Copy className="h-3 w-3" />
                )}
              </Button>
            </div>
          </div>

          <div className="flex justify-end">
            <Button onClick={onClose}>Done</Button>
          </div>
        </div>
      </DialogContent>
    </Dialog>
  );
}
