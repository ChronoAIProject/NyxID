import { useNavigate, useParams, useSearch } from "@tanstack/react-router";
import { useService } from "@/hooks/use-services";
import { SshWebTerminal } from "@/components/dashboard/ssh-web-terminal";
import { Button } from "@/components/ui/button";
import { Badge } from "@/components/ui/badge";
import { Skeleton } from "@/components/ui/skeleton";
import { ArrowLeft, Terminal, X, AlertCircle } from "lucide-react";
import { useCallback } from "react";

export function SshTerminalPage() {
  const { serviceId } = useParams({ strict: false }) as { serviceId: string };
  const search = useSearch({ strict: false }) as { principal?: string };
  const navigate = useNavigate();

  const { data: service, isLoading, error } = useService(serviceId);

  const isSshService = service?.service_type === "ssh";
  const hasCertAuth = service?.ssh_config?.certificate_auth_enabled === true;
  const principal =
    search.principal ??
    service?.ssh_config?.allowed_principals[0] ??
    "ubuntu";
  const targetHost = service?.ssh_config
    ? `${service.ssh_config.host}:${String(service.ssh_config.port)}`
    : null;

  const handleBack = useCallback(() => {
    void navigate({
      to: "/services/$serviceId",
      params: { serviceId },
    });
  }, [navigate, serviceId]);

  const handleDisconnect = useCallback(() => {
    // No-op for now; the terminal shows its own reconnect button.
  }, []);

  if (isLoading) {
    return (
      <div className="flex h-dvh flex-col bg-[#0f172a]">
        <div className="flex items-center gap-3 border-b border-border/30 bg-[#0f172a] px-4 py-2">
          <Skeleton className="h-8 w-8" />
          <Skeleton className="h-5 w-48" />
        </div>
        <div className="flex flex-1 items-center justify-center">
          <Skeleton className="h-96 w-full max-w-4xl" />
        </div>
      </div>
    );
  }

  if (error || !service) {
    return (
      <div className="flex h-dvh flex-col items-center justify-center bg-background">
        <AlertCircle className="mb-4 h-12 w-12 text-muted-foreground/50" />
        <h3 className="mb-2 font-display text-lg font-semibold">
          Service not found
        </h3>
        <p className="mb-4 text-sm text-muted-foreground">
          The service you are looking for does not exist or has been deleted.
        </p>
        <Button variant="outline" onClick={handleBack}>
          Back to Services
        </Button>
      </div>
    );
  }

  if (!isSshService || !hasCertAuth) {
    return (
      <div className="flex h-dvh flex-col items-center justify-center bg-background">
        <AlertCircle className="mb-4 h-12 w-12 text-muted-foreground/50" />
        <h3 className="mb-2 font-display text-lg font-semibold">
          Terminal not available
        </h3>
        <p className="mb-4 max-w-md text-center text-sm text-muted-foreground">
          {!isSshService
            ? "This service is not an SSH service. The web terminal is only available for SSH services."
            : "Certificate authentication is not enabled for this SSH service. Enable it to use the web terminal."}
        </p>
        <Button variant="outline" onClick={handleBack}>
          Back to Service
        </Button>
      </div>
    );
  }

  return (
    <div className="flex h-dvh flex-col bg-[#0f172a]">
      {/* Header bar */}
      <div className="flex shrink-0 items-center gap-3 border-b border-border/30 px-3 py-1.5">
        <Button
          variant="ghost"
          size="icon"
          onClick={handleBack}
          className="h-7 w-7 text-slate-400 hover:text-slate-200"
        >
          <ArrowLeft className="h-4 w-4" />
        </Button>

        <div className="flex items-center gap-2">
          <Terminal className="h-4 w-4 text-slate-400" />
          <span className="text-sm font-medium text-slate-200">
            {service.name}
          </span>
          {targetHost !== null && (
            <span className="text-xs text-slate-500">
              {targetHost}
            </span>
          )}
        </div>

        <Badge variant="accent" className="text-[9px]">
          {principal}
        </Badge>

        <div className="flex-1" />

        <Button
          variant="ghost"
          size="sm"
          onClick={handleBack}
          className="h-7 gap-1 text-xs text-slate-400 hover:text-slate-200"
        >
          <X className="h-3 w-3" />
          Close
        </Button>
      </div>

      {/* Terminal area */}
      <div className="flex-1 overflow-hidden">
        <SshWebTerminal
          serviceId={serviceId}
          principal={principal}
          onDisconnect={handleDisconnect}
        />
      </div>
    </div>
  );
}
