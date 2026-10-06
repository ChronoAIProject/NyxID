import { Link } from "@tanstack/react-router";
import { ArrowUpRight, Copy } from "lucide-react";
import { toast } from "sonner";
import { Button } from "@/components/ui/button";
import { copyToClipboard } from "@/lib/utils";
import { PLUGIN_COMMANDS } from "@/lib/agent-plugins";

export function AgentPluginsSection() {
  async function copyCommand(name: string, command: string) {
    try {
      await copyToClipboard(command);
      toast.success(`${name} install command copied`);
    } catch {
      toast.error("Failed to copy install command");
    }
  }

  return (
    <section aria-labelledby="agent-plugins-heading">
      <div className="mb-3 flex flex-wrap items-baseline justify-between gap-2">
        <h2
          id="agent-plugins-heading"
          className="text-[15px] font-semibold text-foreground"
        >
          Agent plugins
        </h2>
        <Link
          to="/settings"
          search={{ tab: "mcp" }}
          className="text-[11px] text-muted-foreground hover:text-foreground"
        >
          MCP setup{" "}
          <ArrowUpRight className="inline h-3 w-3" aria-hidden="true" />
        </Link>
      </div>
      <div className="grid gap-x-6 gap-y-4 border-y border-border/50 py-4 md:grid-cols-2">
        {PLUGIN_COMMANDS.map(({ name, command }) => (
          <div key={name} className="min-w-0 space-y-2">
            <div className="flex items-center justify-between gap-3">
              <h3 className="text-[12px] font-medium text-foreground">
                {name}
              </h3>
              <Button
                variant="outline"
                size="sm"
                type="button"
                onClick={() => void copyCommand(name, command)}
                aria-label={`Copy ${name} plugin install command`}
              >
                <Copy className="h-3 w-3" aria-hidden="true" />
                Copy command
              </Button>
            </div>
            <pre className="whitespace-pre-wrap break-all font-mono text-[11px] leading-5 text-muted-foreground">
              {command}
            </pre>
          </div>
        ))}
      </div>
    </section>
  );
}
