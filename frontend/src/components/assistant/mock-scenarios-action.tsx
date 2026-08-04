import { Check, FlaskConical, RotateCcw, TriangleAlert, X } from "lucide-react";
import { Button } from "@/components/ui/button";
import {
  Popover,
  PopoverContent,
  PopoverTrigger,
} from "@/components/ui/popover";
import { Switch } from "@/components/ui/switch";
import {
  Tooltip,
  TooltipContent,
  TooltipTrigger,
} from "@/components/ui/tooltip";
import { scenarios } from "@/lib/assistant/scenarios.config";
import { formatRelativeTime } from "@/lib/utils";
import { useAssistantMockScenariosStore } from "@/stores/assistant-mock-scenarios-store";

function scenarioName(id: string): string {
  return id
    .split("-")
    .map((part) => `${part.charAt(0).toUpperCase()}${part.slice(1)}`)
    .join(" ");
}

function activityAge(at: number): string {
  return formatRelativeTime(new Date(at).toISOString());
}

export function MockScenariosAction() {
  const enabled = useAssistantMockScenariosStore((state) => state.enabled);
  const disabledScenarioIds = useAssistantMockScenariosStore(
    (state) => state.disabledScenarioIds,
  );
  const world = useAssistantMockScenariosStore((state) => state.world);
  const engineState = useAssistantMockScenariosStore(
    (state) => state.engineState,
  );
  const lastActivity = useAssistantMockScenariosStore(
    (state) => state.lastActivity,
  );
  const setEnabled = useAssistantMockScenariosStore(
    (state) => state.setEnabled,
  );
  const setScenarioEnabled = useAssistantMockScenariosStore(
    (state) => state.setScenarioEnabled,
  );
  const disconnectService = useAssistantMockScenariosStore(
    (state) => state.disconnectService,
  );
  const resetWorld = useAssistantMockScenariosStore(
    (state) => state.resetWorld,
  );

  return (
    <Popover>
      <Tooltip>
        <TooltipTrigger asChild>
          <PopoverTrigger asChild>
            <Button
              type="button"
              variant="ghost"
              size="icon"
              aria-label="Mock scenarios"
              className="relative shrink-0 text-text-tertiary"
            >
              <FlaskConical className="h-4 w-4" />
              {enabled ? (
                <span
                  aria-hidden="true"
                  data-testid="mock-scenarios-active-dot"
                  className="absolute right-1 top-1 h-1.5 w-1.5 rounded-full bg-nyx-secondary-400 ring-2 ring-background"
                />
              ) : null}
            </Button>
          </PopoverTrigger>
        </TooltipTrigger>
        <TooltipContent>Mock scenarios</TooltipContent>
      </Tooltip>

      <PopoverContent
        align="end"
        sideOffset={6}
        className="w-[min(320px,calc(100vw-24px))] overflow-hidden p-0"
      >
        <section className="space-y-3 p-4">
          <div className="flex items-center justify-between gap-3">
            <div className="min-w-0">
              <h2 className="text-[13px] font-semibold text-foreground">
                Mock scenarios
              </h2>
              {engineState === "loading" ? (
                <p role="status" className="mt-0.5 text-[11px] text-warning">
                  Loading...
                </p>
              ) : engineState === "error" ? (
                <p role="alert" className="mt-0.5 text-[11px] text-destructive">
                  Scenario engine failed to load.
                </p>
              ) : null}
            </div>
            <Switch
              checked={enabled}
              onCheckedChange={setEnabled}
              disabled={engineState === "loading"}
              aria-label="Enable mock scenarios"
            />
          </div>
          <p className="text-[11px] leading-4 text-muted-foreground">
            Intercepts matching chat messages with scripted flows. Session-only;
            other messages reach the assistant normally.
          </p>
          <div className="flex gap-2 text-[11px] leading-4 text-warning">
            <TriangleAlert className="mt-0.5 h-3 w-3 shrink-0" />
            <p>
              Action cards open real connection journeys and can create real
              keys on your account.
            </p>
          </div>
        </section>

        <section
          aria-labelledby="mock-scenario-list-heading"
          className="border-t border-border/60 px-4 py-3"
        >
          <h3
            id="mock-scenario-list-heading"
            className="mb-1 text-[10px] font-semibold uppercase text-text-tertiary"
          >
            Scenarios
          </h3>
          <div className="divide-y divide-border/50">
            {scenarios.map((entry) => {
              const scenarioEnabled = !disabledScenarioIds.includes(entry.id);
              const matched =
                lastActivity?.matched === true &&
                lastActivity.scenarioId === entry.id;
              const expression = `/${entry.pattern.source}/${entry.pattern.flags}`;
              return (
                <div
                  key={entry.id}
                  className="flex min-h-11 items-center gap-3 py-2"
                >
                  <div className="min-w-0 flex-1">
                    <div className="flex min-w-0 items-center gap-1.5">
                      <p className="truncate text-[12px] font-medium text-foreground">
                        {scenarioName(entry.id)}
                      </p>
                      {matched && lastActivity ? (
                        <span
                          data-testid={`scenario-activity-${entry.id}`}
                          className="flex shrink-0 items-center gap-1 text-[10px] text-success"
                        >
                          <Check className="h-3 w-3" />
                          Matched {activityAge(lastActivity.at)}
                        </span>
                      ) : null}
                    </div>
                    <p
                      title={expression}
                      className="truncate font-mono text-[10px] text-text-tertiary"
                    >
                      {expression}
                    </p>
                  </div>
                  <Switch
                    checked={scenarioEnabled}
                    onCheckedChange={(checked) =>
                      setScenarioEnabled(entry.id, checked)
                    }
                    aria-label={`Enable ${entry.id} scenario`}
                  />
                </div>
              );
            })}
          </div>
          {lastActivity && !lastActivity.matched ? (
            <p
              data-testid="unmatched-scenario-activity"
              className="border-t border-border/50 pt-2 text-[10px] text-text-tertiary"
            >
              No scenario matched - {activityAge(lastActivity.at)}; message
              passed through.
            </p>
          ) : null}
        </section>

        <section
          aria-labelledby="mock-scenario-world-heading"
          className="border-t border-border/60 px-4 py-3"
        >
          <div className="flex items-center justify-between gap-3">
            <h3
              id="mock-scenario-world-heading"
              className="text-[10px] font-semibold uppercase text-text-tertiary"
            >
              Connected (mock)
            </h3>
            <Button
              type="button"
              variant="ghost"
              size="sm"
              onClick={resetWorld}
              disabled={world.connected.length === 0}
              className="h-6 px-1.5 text-[10px]"
            >
              <RotateCcw />
              Reset world
            </Button>
          </div>
          {world.connected.length === 0 ? (
            <p className="mt-2 text-[11px] leading-4 text-text-tertiary">
              Nothing connected - <code className="font-mono">need</code> flows
              will run their connect step.
            </p>
          ) : (
            <div className="mt-2 flex flex-wrap gap-1.5">
              {world.connected.map((serviceSlug) => (
                <span
                  key={serviceSlug}
                  className="inline-flex h-6 max-w-full items-center gap-1 rounded-md border border-success/30 bg-success/10 pl-2 pr-1 text-[10px] text-success"
                >
                  <span className="truncate font-mono">{serviceSlug}</span>
                  <button
                    type="button"
                    onClick={() => disconnectService(serviceSlug)}
                    aria-label={`Remove ${serviceSlug} from mock world`}
                    className="flex h-4 w-4 shrink-0 items-center justify-center rounded-[4px] hover:bg-success/15 focus-visible:outline-none"
                  >
                    <X className="h-3 w-3" />
                  </button>
                </span>
              ))}
            </div>
          )}
        </section>

        <footer className="space-y-1 border-t border-border/60 px-4 py-3 text-[10px] leading-4 text-text-tertiary">
          <p>
            Edit flows in{" "}
            <code className="font-mono">
              src/lib/assistant/scenarios.config.ts
            </code>
            .
          </p>
          <p>Mock turns are session-only and disappear on reload.</p>
          <p>Settings use last-write-wins across tabs.</p>
        </footer>
      </PopoverContent>
    </Popover>
  );
}
