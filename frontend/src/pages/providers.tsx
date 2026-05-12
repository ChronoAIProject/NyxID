import { useNavigate } from "@tanstack/react-router";
import { ProviderGrid } from "@/components/dashboard/provider-grid";
import { GatewayInfoCard } from "@/components/dashboard/gateway-info-card";
import { useLlmStatus } from "@/hooks/use-llm-gateway";
import { Button, ButtonIcon } from "@/components/ui/button";
import { Settings } from "lucide-react";

export function ProvidersPage() {
  const navigate = useNavigate();
  const { data: llmStatus } = useLlmStatus();

  return (
    <div className="space-y-8">
      <div className="flex flex-col gap-4 sm:flex-row sm:items-center sm:justify-between">
        <div>
          <h2 className="text-[28px] font-bold leading-none tracking-tight" style={{ letterSpacing: "-0.03em" }}>
            Providers
          </h2>
          <p className="text-[12px] text-muted-foreground">
            Connect your API keys and OAuth accounts for external providers.
          </p>
        </div>
        <Button
          variant="outline"
          className="w-fit"
          onClick={() => void navigate({ to: "/providers/manage" })}
        >
          <ButtonIcon><Settings className="h-3 w-3" /></ButtonIcon>
          Manage Providers
        </Button>
      </div>

      {llmStatus !== undefined && <GatewayInfoCard llmStatus={llmStatus} />}

      <ProviderGrid />
    </div>
  );
}
