import { useUiTranslation } from "@/i18n/use-ui-translation";
import { translateUi } from "@/i18n/use-ui-translation";
import { translateErrorMessage } from "@/lib/error-messages";
import { Link } from "@tanstack/react-router";
import { ArrowUpRight, Copy } from "lucide-react";
import { toast } from "sonner";
import { Button } from "@/components/ui/button";
import { copyToClipboard } from "@/lib/utils";
import { PLUGIN_COMMANDS } from "@/lib/agent-plugins";

export function AgentPluginsSection() {
  useUiTranslation();
  async function copyCommand(name: string, command: string) {
    try {
      await copyToClipboard(command);
      toast.success(translateUi(`${name} install command copied`));
    } catch {
      toast.error(translateErrorMessage("Failed to copy install command"));
    }
  }

  return (
    <section aria-labelledby="agent-plugins-heading">
      <div className="mb-3 flex flex-wrap items-baseline justify-between gap-2">
        <h2
          id="agent-plugins-heading"
          className="type-card-title text-foreground"
        >{translateUi("Agent plugins")}</h2>
        <Link
          to="/settings"
          search={{ tab: "mcp" }}
          className="type-row-title text-text-tertiary hover:text-foreground"
        >{translateUi("MCP setup")}{" "}
          <ArrowUpRight className="inline h-3 w-3" aria-hidden="true" />
        </Link>
      </div>
      <div className="grid gap-x-6 gap-y-4 border-y border-border/50 py-4 md:grid-cols-2">
        {PLUGIN_COMMANDS.map(({ name, command }) => (
          <div key={name} className="min-w-0 space-y-2">
            <div className="flex items-center justify-between gap-3">
              <h3 className="type-row-title text-foreground">{name}</h3>
              <Button
                variant="outline"
                size="sm"
                type="button"
                onClick={() => void copyCommand(name, command)}
                aria-label={translateUi("Copy {{value1}} plugin install command", { value1: name })}
              >
                <Copy className="h-3 w-3" aria-hidden="true" />{translateUi("Copy command")}</Button>
            </div>
            <pre className="whitespace-pre-wrap break-all font-mono text-14 text-muted-foreground">
              {command}
            </pre>
          </div>
        ))}
      </div>
    </section>
  );
}
