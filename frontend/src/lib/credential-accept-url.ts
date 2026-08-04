import { api } from "@/lib/api-client";
import {
  runtimeConfigSchema,
  type RuntimeConfig,
} from "@/schemas/runtime-config";

const DEFAULT_RETURN_TO = "/nodes";

export function safeRelativeReturnTo(returnTo: string): string {
  if (!returnTo || returnTo.length > 2048 || returnTo.includes("\\")) {
    return DEFAULT_RETURN_TO;
  }
  try {
    const url = new URL(returnTo, window.location.origin);
    if (url.origin !== window.location.origin) {
      return DEFAULT_RETURN_TO;
    }
    return `${url.pathname}${url.search}${url.hash}`;
  } catch {
    return DEFAULT_RETURN_TO;
  }
}

async function fetchRuntimeConfig(): Promise<RuntimeConfig> {
  const response = await api.get<unknown>("/runtime-config");
  return runtimeConfigSchema.parse(response);
}

export async function buildStandaloneCredentialAcceptUrl(
  nodeId: string,
  pendingId: string,
  returnTo: string,
): Promise<string> {
  const runtimeConfig = await fetchRuntimeConfig();
  const url = new URL(
    `/nodes/${encodeURIComponent(nodeId)}/credentials/pending/${encodeURIComponent(pendingId)}/accept`,
    runtimeConfig.api_base_url,
  );
  url.searchParams.set("return_to", safeRelativeReturnTo(returnTo));
  return url.href;
}
