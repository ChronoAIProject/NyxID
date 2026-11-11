import { useUiTranslation } from "@/i18n/use-ui-translation";
import { translateUi } from "@/i18n/use-ui-translation";
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
  useUiTranslation();
  const [mode, setMode] = useState<NyxAgentAccessMode>("nyxmode");
  return (
    <div className="space-y-1.5">
      <div role="radiogroup" aria-label={translateUi("Access modes")} className="grid grid-cols-3 gap-1 rounded-lg bg-muted p-1">
        {MODES.map((value) => (
          <button
            key={value}
            type="button"
            role="radio"
            aria-checked={mode === value}
            onClick={() => setMode(value)}
            className={cn(
              "min-w-0 break-words rounded-md px-2 py-1 type-row-title transition-colors focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-input-focus motion-reduce:transition-none",
              mode === value
                ? "bg-card text-foreground shadow-sm"
                : "text-text-tertiary hover:text-foreground",
            )}
          >
            {ACCESS_MODE_LABEL[value]}
          </button>
        ))}
      </div>
      <p className="min-h-[2lh] text-13 text-muted-foreground">
        {ACCESS_MODE_DESCRIPTION[mode]}
        {mode === "nyxmode" ? translateUi(" This is the default.") : ""}
      </p>
    </div>
  );
}
