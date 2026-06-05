import { api } from "@/lib/api-client";
import {
  runtimeConfigSchema,
  type RuntimeConfig,
} from "@/schemas/runtime-config";

function safeRelativeReturnTo(returnTo: string): string {
  const url = new URL(returnTo, window.location.origin);
  if (url.origin !== window.location.origin) {
    return "/nodes";
  }
  return `${url.pathname}${url.search}${url.hash}`;
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
