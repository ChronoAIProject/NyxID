import type { DownstreamService } from "@/types/api";

export function serviceCredentialStatus(
  service?: Pick<DownstreamService, "credential_configured">,
): string {
  if (!service) return "not configured";
  if (service.credential_configured === undefined) return "status unavailable";
  if (service.credential_configured === null) return "stored but unavailable";
  return service.credential_configured ? "configured" : "not configured";
}
