import { useNavigate } from "@tanstack/react-router";
import { useQueryClient } from "@tanstack/react-query";
import {
  Tooltip,
  TooltipContent,
  TooltipTrigger,
} from "@/components/ui/tooltip";
import {
  useAssistantTransportStore,
  type AssistantTransportMode,
} from "@/stores/assistant-transport-store";

const OPTIONS: ReadonlyArray<{
  readonly mode: AssistantTransportMode;
  readonly label: string;
  readonly tooltip: string;
}> = [
  {
    mode: "chat",
    label: "Chat",
    tooltip: "Chat — aevatar conversations API (AG-UI stream).",
  },
  {
    mode: "completions",
    label: "Completions",
    tooltip:
      "Completions — OpenAI-compatible /v1/chat/completions; history kept locally for this session.",
  },
  {
    mode: "workflow",
    label: "Workflow",
    tooltip:
      "Workflow — ad-hoc workflow chat /api/chat; one prompt per run, history kept locally for this session.",
  },
];

/**
 * Switches which aevatar API the chat streams from. The two transports own
 * disjoint conversation worlds, so a change drops the current `?c` selection
 * and re-fetches the assistant queries against the new mode.
 */
export function TransportToggle({
  turnActive,
}: {
  readonly turnActive: boolean;
}) {
  const mode = useAssistantTransportStore((state) => state.mode);
  const setMode = useAssistantTransportStore((state) => state.setMode);
  const queryClient = useQueryClient();
  const navigate = useNavigate();

  function select(next: AssistantTransportMode) {
    if (turnActive || next === mode) return;
    setMode(next);
    void queryClient.invalidateQueries({ queryKey: ["assistant"] });
    void navigate({ to: "/assistant" as never, search: {} as never });
  }

  return (
    <div className="mb-1">
      <div className="px-3 py-1 text-[9px] font-medium uppercase tracking-[1.5px] text-text-tertiary/50">
        Chat API
      </div>
      <div
        role="radiogroup"
        aria-label="Chat API"
        className="flex gap-0.5 rounded-lg bg-overlay p-0.5"
      >
        {OPTIONS.map((option) => {
          const active = option.mode === mode;
          return (
            <Tooltip key={option.mode}>
              <TooltipTrigger asChild>
                <button
                  type="button"
                  role="radio"
                  aria-checked={active}
                  aria-disabled={turnActive}
                  onClick={() => select(option.mode)}
                  className={`flex-1 rounded-lg px-2 py-1 text-[11px] transition-colors ${
                    active
                      ? "bg-overlay-strong font-medium text-foreground"
                      : "text-muted-foreground hover:text-foreground"
                  } ${turnActive ? "cursor-not-allowed opacity-50" : ""}`}
                >
                  {option.label}
                </button>
              </TooltipTrigger>
              <TooltipContent side="right">{option.tooltip}</TooltipContent>
            </Tooltip>
          );
        })}
      </div>
    </div>
  );
}
