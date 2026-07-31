import { useState } from "react";
import { ChevronDown, Copy, Network, Trash2 } from "lucide-react";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import {
  Sheet,
  SheetContent,
  SheetDescription,
  SheetHeader,
  SheetTitle,
  SheetTrigger,
} from "@/components/ui/sheet";
import { Switch } from "@/components/ui/switch";
import {
  Tooltip,
  TooltipContent,
  TooltipTrigger,
} from "@/components/ui/tooltip";
import { useAssistantWireLogStore } from "@/stores/assistant-wire-log-store";
import { canAdminWrite, type User } from "@/types/api";
import { cn } from "@/lib/utils";

function statusVariant(status: number): "success" | "destructive" | "warning" {
  if (status >= 200 && status < 400) return "success";
  if (status >= 500) return "destructive";
  return "warning";
}

function entryJson(entry: ReturnType<typeof useAssistantWireLogStore.getState>["entries"][number]) {
  return JSON.stringify(
    {
      method: entry.method,
      path: entry.path,
      commandType: entry.commandType,
      body: entry.body,
      headers: entry.headers,
      identity: entry.identity,
      status: entry.status,
      truncated: entry.truncated,
    },
    null,
    2,
  );
}

export function AssistantWireLogPanel() {
  const entries = useAssistantWireLogStore((state) => state.entries);
  const captureEnabled = useAssistantWireLogStore(
    (state) => state.captureEnabled,
  );
  const setCaptureEnabled = useAssistantWireLogStore(
    (state) => state.setCaptureEnabled,
  );
  const clear = useAssistantWireLogStore((state) => state.clear);
  const [expanded, setExpanded] = useState<ReadonlySet<string>>(new Set());

  function toggleExpanded(id: string) {
    setExpanded((current) => {
      const next = new Set(current);
      if (next.has(id)) next.delete(id);
      else next.add(id);
      return next;
    });
  }

  return (
    <Sheet>
      <Tooltip>
        <TooltipTrigger asChild>
          <SheetTrigger asChild>
            <Button
              type="button"
              variant="ghost"
              size="icon"
              aria-label="Aevatar wire log"
              className="shrink-0 text-text-tertiary"
            >
              <Network className="h-4 w-4" />
            </Button>
          </SheetTrigger>
        </TooltipTrigger>
        <TooltipContent>Aevatar wire log</TooltipContent>
      </Tooltip>

      <SheetContent
        side="right"
        className="flex w-full flex-col gap-0 p-0 sm:max-w-xl"
      >
        <SheetHeader className="shrink-0 border-b border-border/60 px-5 py-5 pr-12">
          <SheetTitle>Aevatar wire log</SheetTitle>
          <SheetDescription className="sr-only">
            Outbound requests assembled by NyxID for Aevatar.
          </SheetDescription>
        </SheetHeader>

        <div className="flex shrink-0 items-center justify-between gap-4 border-b border-border/60 px-5 py-3">
          <label className="flex items-center gap-2 text-[12px] font-medium text-foreground">
            <Switch
              checked={captureEnabled}
              onCheckedChange={setCaptureEnabled}
              aria-label="Capture Aevatar requests"
            />
            Capture
          </label>
          <Button
            type="button"
            variant="ghost"
            size="sm"
            onClick={clear}
            disabled={entries.length === 0}
          >
            <Trash2 />
            Clear
          </Button>
        </div>

        <div className="min-h-0 flex-1 overflow-y-auto px-3 py-3">
          {entries.length === 0 ? (
            <div className="flex min-h-40 items-center justify-center px-6 text-center text-[12px] text-text-tertiary">
              No captured requests
            </div>
          ) : (
            <div className="space-y-2">
              {[...entries].reverse().map((entry) => {
                const isExpanded = expanded.has(entry.id);
                return (
                  <div
                    key={entry.id}
                    className="overflow-hidden rounded-lg border border-border/70 bg-surface"
                  >
                    <div className="flex items-start gap-2 p-3">
                      <button
                        type="button"
                        onClick={() => toggleExpanded(entry.id)}
                        aria-expanded={isExpanded}
                        className="flex min-w-0 flex-1 items-start gap-2 text-left focus-visible:outline-none"
                      >
                        <ChevronDown
                          className={cn(
                            "mt-0.5 h-3.5 w-3.5 shrink-0 text-text-tertiary transition-transform",
                            isExpanded && "rotate-180",
                          )}
                        />
                        <span className="min-w-0 flex-1">
                          <span className="flex flex-wrap items-center gap-1.5">
                            <span className="text-[10px] tabular-nums text-text-tertiary">
                              {new Date(entry.ts).toLocaleTimeString([], {
                                hour: "2-digit",
                                minute: "2-digit",
                                second: "2-digit",
                              })}
                            </span>
                            {entry.commandType ? (
                              <Badge variant="secondary">
                                {entry.commandType}
                              </Badge>
                            ) : null}
                            <Badge variant={statusVariant(entry.status)}>
                              {entry.status}
                            </Badge>
                          </span>
                          <span className="mt-1 block break-all font-mono text-[11px] leading-5 text-muted-foreground">
                            <strong className="font-semibold text-foreground">
                              {entry.method}
                            </strong>{" "}
                            {entry.path}
                          </span>
                        </span>
                      </button>
                      <Tooltip>
                        <TooltipTrigger asChild>
                          <Button
                            type="button"
                            variant="ghost"
                            size="icon"
                            aria-label={`Copy ${entry.method} ${entry.path} as JSON`}
                            className="h-7 w-7 shrink-0"
                            onClick={() =>
                              void navigator.clipboard.writeText(entryJson(entry))
                            }
                          >
                            <Copy />
                          </Button>
                        </TooltipTrigger>
                        <TooltipContent>Copy as JSON</TooltipContent>
                      </Tooltip>
                    </div>
                    {isExpanded ? (
                      <pre className="max-h-80 overflow-auto border-t border-border/60 bg-background/60 p-3 font-mono text-[10px] leading-5 text-muted-foreground">
                        {entryJson(entry)}
                      </pre>
                    ) : null}
                  </div>
                );
              })}
            </div>
          )}
        </div>

        <p className="shrink-0 border-t border-border/60 px-5 py-3 text-[11px] text-text-tertiary">
          The WebSocket workflow channel is not captured.
        </p>
      </SheetContent>
    </Sheet>
  );
}

export function AssistantWireLogAction({
  user,
}: {
  readonly user: User | null;
}) {
  if (!canAdminWrite(user)) return null;
  return <AssistantWireLogPanel />;
}
