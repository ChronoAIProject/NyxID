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
import { Button } from "@/components/ui/button";
import { ChevronDown, ChevronUp, Zap } from "lucide-react";
import { CopyableField } from "@/components/shared/copyable-field";

interface GatewayInfoCardProps {
  readonly llmStatus: LlmStatusResponse;
}

export function GatewayInfoCard({ llmStatus }: GatewayInfoCardProps) {
  const [expanded, setExpanded] = useState(false);

  const readyProviders = llmStatus.providers.filter(
    (p) => p.status === "ready",
  );

  const exampleCurl = `curl ${llmStatus.gateway_url}/chat/completions \\
  -H "Authorization: Bearer YOUR_NYXID_TOKEN" \\
  -H "Content-Type: application/json" \\
  -d '{
    "model": "gpt-4o",
    "messages": [{"role": "user", "content": "Hello"}]
  }'`;

  return (
    <Card className="border-primary/30 bg-primary/10">
      <CardHeader className="pb-3">
        <div className="flex items-center justify-between">
          <div className="flex items-center gap-2">
            <Zap className="h-5 w-5 text-primary" />
            <div>
              <CardTitle className="text-base">LLM Gateway</CardTitle>
              <CardDescription className="text-xs">
                Route LLM requests through NyxID with your connected provider
                credentials.
              </CardDescription>
            </div>
          </div>
          <div className="flex items-center gap-2">
            {readyProviders.length > 0 && (
              <Badge variant="success">
                {String(readyProviders.length)} provider
                {readyProviders.length === 1 ? "" : "s"} ready
              </Badge>
            )}
            <Button
              variant="ghost"
              size="icon"
              className="h-7 w-7"
              onClick={() => setExpanded((prev) => !prev)}
            >
              {expanded ? (
                <ChevronUp className="h-4 w-4" />
              ) : (
                <ChevronDown className="h-4 w-4" />
              )}
              <span className="sr-only">
                {expanded ? "Collapse" : "Expand"} gateway info
              </span>
            </Button>
          </div>
        </div>
      </CardHeader>

      {expanded && (
        <CardContent className="space-y-4">
          <CopyableField label="Gateway URL" value={llmStatus.gateway_url} />

          <div className="rounded-md border border-border/50 bg-muted/30 p-3">
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
                    variant="outline"
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
            <pre className="rounded bg-muted px-3 py-2 text-[11px] overflow-x-auto whitespace-pre-wrap break-all">
              {exampleCurl}
            </pre>
            <p className="mt-1 text-[10px] text-muted-foreground">
              Replace YOUR_NYXID_TOKEN with your NyxID access token from the
              login response.
            </p>
          </div>
        </CardContent>
      )}
    </Card>
  );
}
