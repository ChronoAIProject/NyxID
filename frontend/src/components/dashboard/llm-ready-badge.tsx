import type { LlmProviderStatus } from "@/types/api";
import { Badge } from "@/components/ui/badge";
import {
  Popover,
  PopoverContent,
  PopoverTrigger,
} from "@/components/ui/popover";
import { CircleCheck } from "lucide-react";
import { CopyableField } from "@/components/shared/copyable-field";

interface LlmReadyBadgeProps {
  readonly llmStatus: LlmProviderStatus;
  readonly gatewayUrl: string;
}

export function LlmReadyBadge({ llmStatus, gatewayUrl }: LlmReadyBadgeProps) {
  const exampleCurl = `curl ${llmStatus.proxy_url}/chat/completions \\
  -H "Authorization: Bearer YOUR_NYXID_TOKEN" \\
  -H "Content-Type: application/json" \\
  -d '{"model": "...", "messages": [{"role": "user", "content": "Hello"}]}'`;

  return (
    <Popover>
      <PopoverTrigger asChild>
        <button type="button" className="cursor-pointer">
          <Badge variant="success" className="gap-1">
            <CircleCheck className="h-3 w-3" />
            LLM Ready
          </Badge>
        </button>
      </PopoverTrigger>
      <PopoverContent className="w-80" align="end">
        <div className="space-y-3">
          <p className="text-xs font-medium">LLM Proxy URLs</p>

          <CopyableField
            label="Direct Proxy URL"
            value={llmStatus.proxy_url}
            size="sm"
          />

          <CopyableField label="Gateway URL" value={gatewayUrl} size="sm" />

          <div>
            <p className="mb-1 text-[10px] font-medium text-muted-foreground">
              Example
            </p>
            <pre className="rounded bg-muted px-2 py-1.5 text-[10px] overflow-x-auto whitespace-pre-wrap break-all">
              {exampleCurl}
            </pre>
            <p className="mt-1 text-[9px] text-muted-foreground">
              Replace YOUR_NYXID_TOKEN with your NyxID access token from the
              login response.
            </p>
          </div>
        </div>
      </PopoverContent>
    </Popover>
  );
}
