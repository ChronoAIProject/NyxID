import { useState } from "react";
import { ACCESS_MODE_DESCRIPTION, ACCESS_MODE_LABEL } from "@/lib/assistant/nyxbot-labels";
import { cn } from "@/lib/utils";
import type { NyxAgentAccessMode } from "@/schemas/assistant-nyxagent";

/*
 * The access-mode explainer for the first-run intro.
 */


const MODES: readonly NyxAgentAccessMode[] = ["nyxmode", "ask_every_time", "autopilot"];

/** The access modes, explained on selection; it changes nothing. */
export function AccessModesPreview() {
  const [mode, setMode] = useState<NyxAgentAccessMode>("nyxmode");
  return (
    <div className="space-y-1.5">
      <div role="radiogroup" aria-label="Access modes" className="grid grid-cols-3 gap-1 rounded-lg border border-border bg-muted p-1">
        {MODES.map((value) => (
          <button
            key={value}
            type="button"
            role="radio"
            aria-checked={mode === value}
            onClick={() => setMode(value)}
            className={cn(
              "rounded-md px-2 py-1 text-11 font-medium transition-colors motion-reduce:transition-none",
              mode === value
                ? "bg-card text-foreground shadow-sm"
                : "text-text-tertiary hover:text-foreground",
            )}
          >
            {ACCESS_MODE_LABEL[value]}
          </button>
        ))}
      </div>
      <p className="min-h-[2lh] text-11 leading-snug text-muted-foreground">
        {ACCESS_MODE_DESCRIPTION[mode]}
        {mode === "nyxmode" ? " This is the default." : ""}
      </p>
    </div>
  );
}
