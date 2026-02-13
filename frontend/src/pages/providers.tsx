import { useNavigate } from "@tanstack/react-router";
import { ProviderGrid } from "@/components/dashboard/provider-grid";
import { GatewayInfoCard } from "@/components/dashboard/gateway-info-card";
import { useLlmStatus } from "@/hooks/use-llm-gateway";
import { Button } from "@/components/ui/button";
import { Settings } from "lucide-react";

export function ProvidersPage() {
  const navigate = useNavigate();
  const { data: llmStatus } = useLlmStatus();

  return (
    <div className="space-y-6">
      <div className="flex items-center justify-between">
        <div>
          <h2 className="font-display text-5xl font-normal tracking-tight">Providers</h2>
          <p className="text-muted-foreground">
            Connect your API keys and OAuth accounts for external providers.
          </p>
        </div>
        <Button
          variant="outline"
          onClick={() => void navigate({ to: "/providers/manage" })}
        >
          <Settings className="mr-2 h-4 w-4" />
          Manage Providers
        </Button>
      </div>

      {llmStatus !== undefined && <GatewayInfoCard llmStatus={llmStatus} />}

      <ProviderGrid />
    </div>
  );
}
