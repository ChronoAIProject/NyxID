import type { DownstreamService } from "@/types/api";

function getServiceAuthKind(
  service: DownstreamService | undefined,
): string | null {
  if (!service) return null;
  return service.auth_type ?? service.auth_method;
}

export function buildNodeCredentialCommand(
  serviceSlug: string,
  service: DownstreamService | undefined,
): string {
  const base = `nyxid-node credentials add --service ${serviceSlug}`;
  if (!service) return `${base} --header Authorization`;

  if (service.auth_method === "query" || service.auth_type === "query") {
    return `${base} --query-param ${service.auth_key_name}`;
  }

  const authKind = getServiceAuthKind(service);
  if (authKind === "bearer" || authKind === "oauth2") {
    return `${base} --header ${service.auth_key_name} --secret-format bearer`;
  }
  if (authKind === "basic") {
    return `${base} --header ${service.auth_key_name} --secret-format basic`;
  }

  return `${base} --header ${service.auth_key_name}`;
}

export function getNodeCredentialPromptHint(
  service: DownstreamService | undefined,
): string | null {
  const authKind = getServiceAuthKind(service);
  if (authKind === "bearer" || authKind === "oauth2") {
    return "When prompted, enter only the raw token. nyxid-node adds the Bearer prefix.";
  }
  if (authKind === "basic") {
    return "When prompted, enter username:password. nyxid-node encodes it as Basic auth.";
  }
  return null;
}
