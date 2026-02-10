import type { ServiceProviderRequirement, ProviderConfig } from "@/types/api";
import { useProviders, useServiceRequirements } from "@/hooks/use-providers";
import { Badge } from "@/components/ui/badge";
import { Skeleton } from "@/components/ui/skeleton";
import { CheckCircle, XCircle, KeyRound } from "lucide-react";

interface ServiceRequirementsViewProps {
  readonly serviceId: string;
  readonly userTokenProviderIds?: ReadonlySet<string>;
}

export function ServiceRequirementsView({
  serviceId,
  userTokenProviderIds,
}: ServiceRequirementsViewProps) {
  const { data: requirements, isLoading: reqLoading } =
    useServiceRequirements(serviceId);
  const { data: providers, isLoading: provLoading } = useProviders();

  const isLoading = reqLoading || provLoading;

  if (isLoading) {
    return <Skeleton className="h-24 w-full" />;
  }

  if (!requirements || requirements.length === 0) {
    return (
      <p className="text-sm text-muted-foreground">
        This service has no provider requirements.
      </p>
    );
  }

  const providerMap = new Map<string, ProviderConfig>(
    providers?.map((p) => [p.id, p]) ?? [],
  );

  return (
    <div className="space-y-2">
      {requirements.map((req) => {
        const provider = providerMap.get(req.provider_config_id);
        const hasToken = userTokenProviderIds?.has(req.provider_config_id) ?? false;

        return (
          <div
            key={req.id}
            className="flex items-center justify-between rounded-md border p-3"
          >
            <div className="flex items-center gap-3">
              <KeyRound className="h-4 w-4 text-muted-foreground" />
              <div>
                <p className="text-sm font-medium">
                  {provider?.name ?? req.provider_name}
                </p>
                <p className="text-xs text-muted-foreground">
                  {req.injection_method === "bearer"
                    ? "Bearer token"
                    : req.injection_method === "header"
                      ? `Header: ${req.injection_key ?? ""}`
                      : `Query: ${req.injection_key ?? ""}`}
                </p>
              </div>
            </div>
            <div className="flex items-center gap-2">
              <Badge variant={req.required ? "default" : "secondary"}>
                {req.required ? "Required" : "Optional"}
              </Badge>
              {userTokenProviderIds !== undefined &&
                (hasToken ? (
                  <CheckCircle className="h-4 w-4 text-emerald-500" />
                ) : (
                  <XCircle className="h-4 w-4 text-destructive" />
                ))}
            </div>
          </div>
        );
      })}
    </div>
  );
}
