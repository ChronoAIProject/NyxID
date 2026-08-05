import type { ConnectLinkPreview } from "@/schemas/connect-links";

export function connectLinkErrorMessage(error: unknown): string {
  return error instanceof Error ? error.message : "The connection request failed.";
}

export function connectLinkNeedsOAuthCredentials(
  preview: ConnectLinkPreview,
): boolean {
  return (
    (preview.connect_method === "oauth" ||
      preview.connect_method === "device_code") &&
    (preview.credential_mode === "user" ||
      (preview.credential_mode === "both" &&
        !preview.has_platform_oauth_credentials))
  );
}

export function connectLinkNeedsSetupForm(preview: ConnectLinkPreview): boolean {
  return (
    preview.connect_method === "api_key" ||
    preview.requires_gateway_url ||
    connectLinkNeedsOAuthCredentials(preview)
  );
}

export function connectLinkProviderError(search: string): string | null {
  const params = new URLSearchParams(search);
  if (params.get("provider_status") !== "error") return null;
  return params.get("message") ?? "Provider authorization was not completed.";
}
