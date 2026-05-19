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

interface GatewayInfoCardProps {
  readonly llmStatus: LlmStatusResponse;
}

export function GatewayInfoCard({ llmStatus }: GatewayInfoCardProps) {
  const readyProviders = llmStatus.providers.filter(
    (p) => p.status === "ready",
  );

  const gatewayUrl = llmStatus.gateway_url || window.location.origin + "/api/v1/llm";

  const exampleCurl = `curl ${gatewayUrl}/chat/completions \\
  -H "Authorization: Bearer YOUR_NYXID_TOKEN" \\
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
          <p className="mb-1 text-xs font-medium text-muted-foreground">
            Example Request
          </p>
          <pre className="rounded-xl border border-border bg-muted px-3 py-2 text-[11px] overflow-x-auto whitespace-pre-wrap break-all">
            {exampleCurl}
          </pre>
          <p className="mt-1 text-[10px] text-muted-foreground">
            Replace YOUR_NYXID_TOKEN with your NyxID access token from the
            login response.
          </p>
        </div>
      </CardContent>
    </Card>
  );
}
