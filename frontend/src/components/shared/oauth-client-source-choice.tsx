import { KeyRound } from "lucide-react";
import { NyxidIcon } from "@/components/brand/nyxid-icon";
import { Label } from "@/components/ui/label";
import {
  Tooltip,
  TooltipContent,
  TooltipProvider,
  TooltipTrigger,
} from "@/components/ui/tooltip";
import { cn } from "@/lib/utils";

export type OAuthClientSource = "managed" | "self";

/**
 * Which OAuth app authorizes a `both`-mode connection: NyxID's own app (one
 * click, curated scopes) or the user's own client credentials. Shared by the
 * dashboard's add-service dialog and the CLI wizard so both read the same.
 */
export function OAuthClientSourceChoice({
  value,
  onChange,
  disabled = false,
}: {
  readonly value: OAuthClientSource;
  readonly onChange: (source: OAuthClientSource) => void;
  readonly disabled?: boolean;
}) {
  return (
    <div className="space-y-3">
      <Label>Which OAuth app authorizes this?</Label>
      {/* The CLI wizard has no app-level provider, so carry our own. */}
      <TooltipProvider delayDuration={200}>
        <div
          role="radiogroup"
          aria-label="OAuth client"
          className="grid grid-cols-2 gap-3"
        >
          <Tooltip>
            <TooltipTrigger asChild>
              <SourceCard
                checked={value === "managed"}
                disabled={disabled}
                onSelect={() => onChange("managed")}
                icon={<NyxidIcon className="h-5 w-5" alt="" />}
                title="NyxID managed"
                description="One click. NyxID’s OAuth app handles authorization. Curated scopes."
              />
            </TooltipTrigger>
            <TooltipContent side="bottom" className="max-w-64">
              The provider’s consent screen shows NyxID’s own app. NyxID keeps
              the token encrypted and refreshes it for you. Only the scopes
              NyxID’s app is approved for are available.
            </TooltipContent>
          </Tooltip>
          <SourceCard
            checked={value === "self"}
            disabled={disabled}
            onSelect={() => onChange("self")}
            icon={<KeyRound className="h-5 w-5" />}
            title="Your own OAuth app"
            description="Use your own client credentials. Full scope control."
          />
        </div>
      </TooltipProvider>
    </div>
  );
}

/** Forwards the tooltip trigger's ref and handlers so the card can host it. */
function SourceCard({
  checked,
  disabled,
  onSelect,
  icon,
  title,
  description,
  onClick,
  ...triggerProps
}: {
  readonly checked: boolean;
  readonly disabled: boolean;
  readonly onSelect: () => void;
  readonly icon: React.ReactNode;
  readonly title: string;
  readonly description: string;
} & React.ComponentPropsWithRef<"button">) {
  return (
    <button
      {...triggerProps}
      type="button"
      role="radio"
      aria-checked={checked}
      disabled={disabled}
      onClick={(event) => {
        onClick?.(event);
        onSelect();
      }}
      className={cn(
        "flex flex-col items-center gap-2 rounded-lg border-2 p-4 text-center transition-colors duration-300 disabled:cursor-not-allowed disabled:opacity-60",
        checked
          ? "border-primary bg-primary/5"
          : "border-border hover:border-hairline-strong",
      )}
    >
      {icon}
      <span className="text-xs font-medium">{title}</span>
      <span className="text-10 text-muted-foreground">{description}</span>
    </button>
  );
}
