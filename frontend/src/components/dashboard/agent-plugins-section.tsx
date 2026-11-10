import { translateUi, useUiTranslation } from "@/i18n/use-ui-translation";

import { Link } from "@tanstack/react-router";
import { ArrowUpRight, Copy } from "lucide-react";
import { toast } from "sonner";
import { Button } from "@/components/ui/button";
import { copyToClipboard } from "@/lib/utils";
import { PLUGIN_COMMANDS } from "@/lib/agent-plugins";

export function AgentPluginsSection() {
  const { t } = useUiTranslation();
  async function copyCommand(name: string, command: string) {
    try {
      await copyToClipboard(command);
      toast.success(translateUi("{{name}} install command copied", { name: name }));
    } catch {
      toast.error(translateUi("Failed to copy install command"));
    }
  }

  return (
    <section aria-labelledby="agent-plugins-heading">
      <div className="mb-3 flex flex-wrap items-baseline justify-between gap-2">
        <h2
          id="agent-plugins-heading"
          className="text-15 font-semibold text-foreground"
        >{t("Agent plugins")}</h2>
        <Link
          to="/settings"
          search={{ tab: "mcp" }}
          className="text-11 text-text-tertiary hover:text-foreground"
        >{t("MCP setup")}{" "}
          <ArrowUpRight className="inline h-3 w-3" aria-hidden="true" />
        </Link>
      </div>
      <div className="grid gap-x-6 gap-y-4 border-y border-border/50 py-4 md:grid-cols-2">
        {PLUGIN_COMMANDS.map(({ name, command }) => (
          <div key={name} className="min-w-0 space-y-2">
            <div className="flex items-center justify-between gap-3">
              <h3 className="text-12 font-medium text-foreground">{name}</h3>
              <Button
                variant="outline"
                size="sm"
                type="button"
                onClick={() => void copyCommand(name, command)}
                aria-label={t("Copy {{name}} plugin install command", { name: name })}
              >
                <Copy className="h-3 w-3" aria-hidden="true" />{t("Copy command")}</Button>
            </div>
            <pre className="whitespace-pre-wrap break-all font-mono text-11 leading-5 text-muted-foreground">
              {command}
            </pre>
          </div>
        ))}
      </div>
    </section>
  );
}
