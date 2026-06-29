import { useState } from "react";
import type { LlmStatusResponse } from "@/types/api";
import {
  Card,
  CardContent,
  CardDescription,
  CardHeader,
  CardTitle,
} from "@/components/ui/card";
import { Badge } from "@/components/ui/badge";
import { CopyableField } from "@/components/shared/copyable-field";
import {
  AgentKeyPicker,
  type PickedAgentKey,
} from "@/components/dashboard/agent-key-picker";

interface GatewayInfoCardProps {
  readonly llmStatus: LlmStatusResponse;
}

export function GatewayInfoCard({ llmStatus }: GatewayInfoCardProps) {
  const readyProviders = llmStatus.providers.filter(
    (p) => p.status === "ready",
  );

  const gatewayUrl = llmStatus.gateway_url || window.location.origin + "/api/v1/llm";

  // Mirrors LlmReadyBadge: null while keys load OR when the user has zero
  // Agent Keys. The picker renders the "Create your first Agent Key →"
  // affordance in the zero-keys case.
  const [pickedKey, setPickedKey] = useState<PickedAgentKey | null>(null);

  // `key_prefix` is the server-issued preview (never the raw secret). We
  // append `••••••••` so the redaction is visually explicit.
  const tokenPreview = pickedKey?.preview ?? "YOUR_NYXID_TOKEN";

  const exampleCurl = `curl ${gatewayUrl}/chat/completions \\
  -H "Authorization: Bearer ${tokenPreview}" \\
  -H "Content-Type: application/json" \\
  -d '{
    "model": "gpt-4o",
    "messages": [{"role": "user", "content": "Hello"}]
  }'`;

  return (
    <Card>
      <CardHeader className="pb-3">
        <div className="flex items-start justify-between gap-3">
          <div className="min-w-0">
            <CardTitle className="text-base">LLM Gateway</CardTitle>
            <CardDescription className="text-xs">
              Route LLM requests through NyxID with your connected provider
              credentials.
            </CardDescription>
          </div>
          <div className="flex shrink-0 items-center gap-2">
            {readyProviders.length > 0 && (
              <Badge
                variant="success"
                className="hidden whitespace-nowrap sm:inline-flex"
              >
                {String(readyProviders.length)} provider
                {readyProviders.length === 1 ? "" : "s"} ready
              </Badge>
            )}
          </div>
        </div>
        {readyProviders.length > 0 && (
          <Badge variant="success" className="mt-2 w-fit sm:hidden">
            {String(readyProviders.length)} provider
            {readyProviders.length === 1 ? "" : "s"} ready
          </Badge>
        )}
      </CardHeader>

      <CardContent className="space-y-4">
        <CopyableField label="Gateway URL" value={gatewayUrl} />

        <div className="rounded-xl border border-border/50 bg-muted/30 p-3">
          <p className="text-xs text-muted-foreground">
            The gateway accepts OpenAI-compatible requests and routes them to
            the correct provider based on the model name. Your provider
            credentials are injected server-side.
          </p>
        </div>

        {readyProviders.length > 0 && (
          <div>
            <p className="mb-2 text-xs font-medium text-muted-foreground">
              Ready Providers
            </p>
            <div className="flex flex-wrap gap-1.5">
              {readyProviders.map((p) => (
                <Badge
                  key={p.provider_slug}
                  variant="secondary"
                  className="text-xs"
                >
                  {p.provider_name}
                </Badge>
              ))}
            </div>
          </div>
        )}

        <div>
          <p className="mb-2 text-xs font-medium text-muted-foreground">
            Agent Key
          </p>
          <AgentKeyPicker onSelect={setPickedKey} />
        </div>

        <div>
          <p className="mb-1 text-xs font-medium text-muted-foreground">
            Example Request
          </p>
          <pre className="rounded-xl border border-border bg-muted px-3 py-2 text-[11px] overflow-x-auto whitespace-pre-wrap break-all">
            {exampleCurl}
          </pre>
          <p className="mt-1 text-[10px] text-muted-foreground">
            {pickedKey
              ? "Using the picked Agent Key preview. Copy the full key from the Agent Keys tab to run this."
              : "Create an Agent Key in the Agent Keys tab, then pick it above to see a ready-to-run example."}
          </p>
        </div>
      </CardContent>
    </Card>
  );
}
