import { useState } from "react";
import type { LlmProviderStatus } from "@/types/api";
import { Badge } from "@/components/ui/badge";
import {
  Popover,
  PopoverContent,
  PopoverTrigger,
} from "@/components/ui/popover";
import { CopyableField } from "@/components/shared/copyable-field";
import {
  AgentKeyPicker,
  type PickedAgentKey,
} from "@/components/dashboard/agent-key-picker";

interface LlmReadyBadgeProps {
  readonly llmStatus: LlmProviderStatus;
  readonly gatewayUrl: string;
}

export function LlmReadyBadge({ llmStatus, gatewayUrl }: LlmReadyBadgeProps) {
  // `pickedKey` is null while keys are still loading OR once they have
  // loaded but the user has zero Agent Keys. In the zero-keys case the
  // picker itself renders a "Create your first Agent Key →" affordance,
  // and we keep the curl example on a placeholder token.
  const [pickedKey, setPickedKey] = useState<PickedAgentKey | null>(null);

  // `key_prefix` is the server-issued preview (e.g. `nyxid_ag_ab12`); it is
  // never the raw secret. We append `••••••••` to make the redaction
  // explicit in the rendered output.
  const tokenPreview = pickedKey?.preview ?? "YOUR_NYXID_TOKEN";

  const exampleCurl = `curl ${llmStatus.proxy_url}/chat/completions \\
  -H "Authorization: Bearer ${tokenPreview}" \\
  -H "Content-Type: application/json" \\
  -d '{"model": "...", "messages": [{"role": "user", "content": "Hello"}]}'`;

  return (
    <Popover>
      <PopoverTrigger asChild>
        <button type="button" className="cursor-pointer">
          <Badge variant="success">LLM Ready</Badge>
        </button>
      </PopoverTrigger>
      <PopoverContent className="w-80" align="end">
        <div className="space-y-3">
          <p className="text-xs font-medium">LLM Proxy URLs</p>

          <CopyableField
            label="Direct Proxy URL"
            value={llmStatus.proxy_url}
          />

          <CopyableField label="Gateway URL" value={gatewayUrl} size="sm" />

          <div>
            <p className="mb-1 text-[10px] font-medium text-muted-foreground">
              Agent Key
            </p>
            <AgentKeyPicker onSelect={setPickedKey} />
          </div>

          <div>
            <p className="mb-1 text-[10px] font-medium text-muted-foreground">
              Example
            </p>
            <pre className="rounded bg-muted px-2 py-1.5 text-[10px] overflow-x-auto whitespace-pre-wrap break-all">
              {exampleCurl}
            </pre>
            <p className="mt-1 text-[9px] text-muted-foreground">
              {pickedKey
                ? "Using the picked Agent Key preview. Copy the full key from the Agent Keys tab to run this."
                : "Create an Agent Key in the Agent Keys tab, then pick it above to see a ready-to-run example."}
            </p>
          </div>
        </div>
      </PopoverContent>
    </Popover>
  );
}
