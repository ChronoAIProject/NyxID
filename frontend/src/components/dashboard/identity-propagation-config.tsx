import { Label } from "@/components/ui/label";
import { Switch } from "@/components/ui/switch";
import { Input } from "@/components/ui/input";
import { Badge } from "@/components/ui/badge";

const PROPAGATION_MODES = [
  { value: "none", label: "None", description: "No identity propagation" },
  { value: "headers", label: "Headers", description: "X-NyxID-* headers" },
  { value: "jwt", label: "JWT", description: "Signed JWT assertion" },
  { value: "both", label: "Both", description: "Headers + JWT" },
] as const;

interface IdentityPropagationConfigProps {
  readonly mode: string;
  readonly includeUserId: boolean;
  readonly includeEmail: boolean;
  readonly includeName: boolean;
  readonly jwtAudience: string;
  readonly onModeChange: (mode: string) => void;
  readonly onIncludeUserIdChange: (value: boolean) => void;
  readonly onIncludeEmailChange: (value: boolean) => void;
  readonly onIncludeNameChange: (value: boolean) => void;
  readonly onJwtAudienceChange: (value: string) => void;
}

export function IdentityPropagationConfig({
  mode,
  includeUserId,
  includeEmail,
  includeName,
  jwtAudience,
  onModeChange,
  onIncludeUserIdChange,
  onIncludeEmailChange,
  onIncludeNameChange,
  onJwtAudienceChange,
}: IdentityPropagationConfigProps) {
  const showFieldToggles = mode !== "none";
  const showJwtAudience = mode === "jwt" || mode === "both";

  return (
    <div className="space-y-4">
      <div className="space-y-2">
        <Label>Propagation Mode</Label>
        <div className="flex flex-wrap gap-2">
          {PROPAGATION_MODES.map((option) => (
            <button
              key={option.value}
              type="button"
              onClick={() => onModeChange(option.value)}
              className="focus:outline-none"
            >
              <Badge
                variant={mode === option.value ? "default" : "outline"}
                className="cursor-pointer px-3 py-1.5"
              >
                {option.label}
              </Badge>
            </button>
          ))}
        </div>
        <p className="text-xs text-muted-foreground">
          {PROPAGATION_MODES.find((m) => m.value === mode)?.description ??
            "Select how user identity is forwarded to this service"}
        </p>
      </div>

      {showFieldToggles && (
        <div className="space-y-3 rounded-md border p-3">
          <p className="text-sm font-medium">Include Fields</p>
          <div className="flex items-center justify-between">
            <Label htmlFor="include-user-id" className="text-sm font-normal">
              User ID
            </Label>
            <Switch
              id="include-user-id"
              checked={includeUserId}
              onCheckedChange={onIncludeUserIdChange}
            />
          </div>
          <div className="flex items-center justify-between">
            <Label htmlFor="include-email" className="text-sm font-normal">
              Email
            </Label>
            <Switch
              id="include-email"
              checked={includeEmail}
              onCheckedChange={onIncludeEmailChange}
            />
          </div>
          <div className="flex items-center justify-between">
            <Label htmlFor="include-name" className="text-sm font-normal">
              Display Name
            </Label>
            <Switch
              id="include-name"
              checked={includeName}
              onCheckedChange={onIncludeNameChange}
            />
          </div>
        </div>
      )}

      {showJwtAudience && (
        <div className="space-y-2">
          <Label htmlFor="jwt-audience">JWT Audience (optional)</Label>
          <Input
            id="jwt-audience"
            placeholder="Defaults to service base URL"
            value={jwtAudience}
            onChange={(e) => onJwtAudienceChange(e.target.value)}
            maxLength={500}
          />
          <p className="text-xs text-muted-foreground">
            Custom audience claim for the identity JWT. Defaults to the
            service base URL if left empty.
          </p>
        </div>
      )}
    </div>
  );
}
