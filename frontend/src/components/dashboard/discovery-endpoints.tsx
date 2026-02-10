import { copyToClipboard } from "@/lib/utils";
import { Button } from "@/components/ui/button";
import { Copy } from "lucide-react";
import { toast } from "sonner";

interface DiscoveryEndpointsProps {
  readonly issuer: string;
  readonly authorizationEndpoint: string;
  readonly tokenEndpoint: string;
  readonly userinfoEndpoint: string;
  readonly jwksUri: string;
}

async function handleCopy(text: string, label: string) {
  try {
    await copyToClipboard(text);
    toast.success(`${label} copied to clipboard`);
  } catch {
    toast.error("Failed to copy to clipboard");
  }
}

export function DiscoveryEndpoints({
  issuer,
  authorizationEndpoint,
  tokenEndpoint,
  userinfoEndpoint,
  jwksUri,
}: DiscoveryEndpointsProps) {
  const endpoints = [
    { label: "Issuer", value: issuer },
    { label: "Authorization", value: authorizationEndpoint },
    { label: "Token", value: tokenEndpoint },
    { label: "UserInfo", value: userinfoEndpoint },
    { label: "JWKS", value: jwksUri },
  ] as const;

  return (
    <div>
      <p className="mb-2 text-xs font-medium text-muted-foreground">
        Discovery Endpoints
      </p>
      <div className="space-y-1">
        {endpoints.map((ep) => (
          <div
            key={ep.label}
            className="flex items-center justify-between text-xs"
          >
            <span className="text-muted-foreground">{ep.label}</span>
            <div className="flex items-center gap-1">
              <code className="max-w-[300px] truncate rounded bg-muted px-1.5 py-0.5">
                {ep.value}
              </code>
              <Button
                variant="ghost"
                size="icon"
                className="h-5 w-5"
                onClick={() => void handleCopy(ep.value, ep.label)}
              >
                <Copy className="h-2.5 w-2.5" />
              </Button>
            </div>
          </div>
        ))}
      </div>
    </div>
  );
}
