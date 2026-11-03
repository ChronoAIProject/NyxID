import { KeyRound } from "lucide-react";
import { NyxidIcon } from "@/components/brand/nyxid-icon";
import { ChoiceCard, ChoiceCardGroup } from "@/components/shared/choice-card";
import {
  Tooltip,
  TooltipContent,
  TooltipProvider,
  TooltipTrigger,
} from "@/components/ui/tooltip";

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
    // The CLI wizard has no app-level provider, so carry our own.
    <TooltipProvider delayDuration={200}>
      <ChoiceCardGroup
        label="Which OAuth app authorizes this?"
        ariaLabel="OAuth client"
      >
        <Tooltip>
          <TooltipTrigger asChild>
            <ChoiceCard
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
        <ChoiceCard
          checked={value === "self"}
          disabled={disabled}
          onSelect={() => onChange("self")}
          icon={<KeyRound className="h-5 w-5" />}
          title="Your own OAuth app"
          description="Use your own client credentials. Full scope control."
        />
      </ChoiceCardGroup>
    </TooltipProvider>
  );
}
