import { useUiTranslation } from "@/i18n/use-ui-translation";
import { translateUi } from "@/i18n/use-ui-translation";
import { translateErrorMessage } from "@/lib/error-messages";
import { useState } from "react";
import {
  useSaConnections,
  useConnectSaService,
  useUpdateSaConnectionCredential,
  useDisconnectSaService,
} from "@/hooks/use-service-accounts";
import { useServices } from "@/hooks/use-services";
import { formatDate } from "@/lib/utils";
import { ApiError } from "@/lib/api-client";
import { CredentialDialog } from "@/components/dashboard/credential-dialog";
import type { DownstreamService } from "@/types/api";
import { DetailSection } from "@/components/shared/detail-section";
import { Skeleton } from "@/components/ui/skeleton";
import { Button, ButtonIcon } from "@/components/ui/button";
import { Badge } from "@/components/ui/badge";
import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from "@/components/ui/table";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu";
import { Plug, Unlink, KeyRound } from "lucide-react";
import { ServiceIcon } from "@/components/service-icon";
import { toast } from "sonner";

interface SaConnectedServicesProps {
  readonly saId: string;
}

export function SaConnectedServices({ saId }: SaConnectedServicesProps) {
  useUiTranslation();
  const { data: saConnections, isLoading: connectionsLoading } =
    useSaConnections(saId);
  const { data: allServices } = useServices();
  const connectServiceMutation = useConnectSaService();
  const updateConnectionCredentialMutation = useUpdateSaConnectionCredential();
  const disconnectServiceMutation = useDisconnectSaService();

  const connectedServiceIds = new Set(
    saConnections?.map((c) => c.service_id) ?? [],
  );
  const availableServices = (allServices ?? []).filter(
    (s) =>
      s.is_active &&
      s.service_type === "http" &&
      s.service_category !== "provider" &&
      !connectedServiceIds.has(s.id),
  );

  const [serviceCredentialDialog, setServiceCredentialDialog] = useState<{
    readonly service: DownstreamService;
    readonly mode: "connect" | "update";
  } | null>(null);

  function handleConnectService(service: DownstreamService) {
    if (service.requires_user_credential) {
      setServiceCredentialDialog({ service, mode: "connect" });
    } else {
      void handleConnectServiceDirect(service.id);
    }
  }

  async function handleConnectServiceDirect(serviceId: string) {
    try {
      await connectServiceMutation.mutateAsync({ saId, serviceId });
      toast.success(translateUi("Service connected"));
    } catch (err) {
      if (err instanceof ApiError) {
        toast.error(translateErrorMessage(err.message));
      } else {
        toast.error(translateErrorMessage("Failed to connect service"));
      }
    }
  }

  async function handleServiceCredentialSubmit(
    credential: string,
    label?: string,
  ) {
    if (!serviceCredentialDialog) return;
    const { service, mode } = serviceCredentialDialog;

    try {
      if (mode === "connect") {
        await connectServiceMutation.mutateAsync({
          saId,
          serviceId: service.id,
          credential,
          credentialLabel: label,
        });
        toast.success(translateUi("Connected to {{value1}}", { value1: service.name }));
      } else {
        await updateConnectionCredentialMutation.mutateAsync({
          saId,
          serviceId: service.id,
          credential,
          credentialLabel: label,
        });
        toast.success(translateUi("Credential updated"));
      }
      setServiceCredentialDialog(null);
    } catch (err) {
      if (err instanceof ApiError) {
        toast.error(translateErrorMessage(err.message));
      } else {
        toast.error(
          translateErrorMessage(mode === "connect"
            ? "Failed to connect service"
            : "Failed to update credential"),
        );
      }
    }
  }

  function handleUpdateServiceCredential(serviceId: string) {
    const service = allServices?.find((s) => s.id === serviceId);
    if (service) {
      setServiceCredentialDialog({ service, mode: "update" });
    }
  }

  async function handleDisconnectService(serviceId: string) {
    try {
      await disconnectServiceMutation.mutateAsync({ saId, serviceId });
      toast.success(translateUi("Service disconnected"));
    } catch (err) {
      if (err instanceof ApiError) {
        toast.error(translateErrorMessage(err.message));
      } else {
        toast.error(translateErrorMessage("Failed to disconnect service"));
      }
    }
  }

  return (
    <>
      <DetailSection title={translateUi("Connected Services")}>
        <div className="space-y-3 p-4">
        {connectionsLoading ? (
          <Skeleton className="h-24 w-full" />
        ) : saConnections && saConnections.length > 0 ? (
          <div className="rounded-xl border border-border/50 bg-card overflow-hidden">
            <Table>
              <TableHeader>
                <TableRow>
                  <TableHead>{translateUi("Service")}</TableHead>
                  <TableHead>{translateUi("Category")}</TableHead>
                  <TableHead>{translateUi("Auth Type")}</TableHead>
                  <TableHead>{translateUi("Credential")}</TableHead>
                  <TableHead>{translateUi("Label")}</TableHead>
                  <TableHead>{translateUi("Connected")}</TableHead>
                  <TableHead>{translateUi("Actions")}</TableHead>
                </TableRow>
              </TableHeader>
              <TableBody>
                {saConnections.map((conn) => (
                  <TableRow key={conn.service_id}>
                    <TableCell className="font-normal">
                      {conn.service_name}
                    </TableCell>
                    <TableCell>
                      <Badge variant="secondary">{conn.service_category}</Badge>
                    </TableCell>
                    <TableCell className="text-muted-foreground">
                      {conn.auth_type ?? "-"}
                    </TableCell>
                    <TableCell>
                      <Badge
                        variant={conn.has_credential ? "success" : "secondary"}
                      >
                        {conn.has_credential ? translateUi("Stored") : translateUi("None")}
                      </Badge>
                    </TableCell>
                    <TableCell className="text-muted-foreground">
                      {conn.credential_label ?? "-"}
                    </TableCell>
                    <TableCell className="text-muted-foreground">
                      {formatDate(conn.connected_at)}
                    </TableCell>
                    <TableCell>
                      <div className="flex justify-end gap-1">
                        {conn.has_credential && (
                          <Button
                            variant="ghost"
                            onClick={() =>
                              handleUpdateServiceCredential(conn.service_id)
                            }
                            disabled={
                              updateConnectionCredentialMutation.isPending
                            }
                          >
                            <ButtonIcon><KeyRound className="h-3 w-3" /></ButtonIcon>{translateUi("Update")}</Button>
                        )}
                        <Button
                          variant="ghost"
                          onClick={() =>
                            void handleDisconnectService(conn.service_id)
                          }
                          disabled={disconnectServiceMutation.isPending}
                        >
                          <ButtonIcon><Unlink className="h-3 w-3" /></ButtonIcon>{translateUi("Disconnect")}</Button>
                      </div>
                    </TableCell>
                  </TableRow>
                ))}
              </TableBody>
            </Table>
          </div>
        ) : (
          <p className="text-14 text-muted-foreground">{translateUi("No services connected to this service account.")}</p>
        )}

        {availableServices.length > 0 && (
          <ConnectServiceDropdown
            services={availableServices}
            onSelect={handleConnectService}
          />
        )}
        </div>
      </DetailSection>

      {serviceCredentialDialog !== null && (
        <CredentialDialog
          service={serviceCredentialDialog.service}
          mode={serviceCredentialDialog.mode}
          onSubmit={(credential, label) =>
            void handleServiceCredentialSubmit(credential, label)
          }
          onCancel={() => setServiceCredentialDialog(null)}
          isPending={
            connectServiceMutation.isPending ||
            updateConnectionCredentialMutation.isPending
          }
        />
      )}
    </>
  );
}

function ConnectServiceDropdown({
  services,
  onSelect,
}: {
  readonly services: readonly DownstreamService[];
  readonly onSelect: (service: DownstreamService) => void;
}) {
  useUiTranslation();
  return (
    <DropdownMenu>
      <DropdownMenuTrigger asChild>
        <Button variant="outline">
          <ButtonIcon><Plug className="h-3 w-3" /></ButtonIcon>{translateUi("Connect Service")}</Button>
      </DropdownMenuTrigger>
      <DropdownMenuContent style={{ maxHeight: "16rem", overflowY: "auto" }}>
        {services.map((s) => (
          <DropdownMenuItem key={s.id} onClick={() => onSelect(s)}>
            <ServiceIcon slug={s.slug} size="2xs" />
            <span>{s.name}</span>
            {s.requires_user_credential && (
              <KeyRound className="ml-1 h-3 w-3 text-muted-foreground" />
            )}
            <Badge variant="secondary" className="ml-auto text-12">
              {s.service_category}
            </Badge>
          </DropdownMenuItem>
        ))}
      </DropdownMenuContent>
    </DropdownMenu>
  );
}
