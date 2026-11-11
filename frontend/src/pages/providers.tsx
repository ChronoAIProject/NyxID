import { useUiTranslation } from "@/i18n/use-ui-translation";
import { useNavigate } from "@tanstack/react-router";
import { ProviderGrid } from "@/components/dashboard/provider-grid";
import { GatewayInfoCard } from "@/components/dashboard/gateway-info-card";
import { useLlmStatus } from "@/hooks/use-llm-gateway";
import { PageHeader } from "@/components/shared/page-header";
import { AddCtaButton } from "@/components/shared/add-cta-button";
import { Settings } from "lucide-react";

export function ProvidersPage() {
  const { t } = useUiTranslation();
  const navigate = useNavigate();
  const { data: llmStatus } = useLlmStatus();

  return (
    <div className="space-y-8">
      <PageHeader
        title={t("Providers")}
        description={t("Connect your API keys and OAuth accounts for external providers.")}
        actions={
          <AddCtaButton
            label={t("Manage Providers")}
            icon={Settings}
            onClick={() => void navigate({ to: "/providers/manage" })}
          />
        }
      />

      {llmStatus !== undefined && <GatewayInfoCard llmStatus={llmStatus} />}

      <ProviderGrid />
    </div>
  );
}
