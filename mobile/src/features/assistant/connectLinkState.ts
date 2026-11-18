import type {
  ConnectLinkPreview,
  CompleteConnectLinkInput,
  CompleteConnectLinkResponse,
} from "./connectLinkContracts";
import {
  connectCredentialFormSchema,
  connectOAuthFormSchema,
  validateConnectCredentialForm,
  validateConnectOAuthForm,
} from "./connectLinkContracts";
import { isApiError } from "../../lib/api/ApiError";

export function connectNeedsOAuthCredentials(
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
export function connectShowsEndpoint(preview: ConnectLinkPreview): boolean {
  return preview.requires_gateway_url || !!preview.endpoint_url;
}
export function connectFormValues(
  preview: ConnectLinkPreview,
  platform: boolean,
  raw: {
    credential: string;
    endpoint_url: string;
    oauth_client_id: string;
    oauth_client_secret: string;
  },
): CompleteConnectLinkInput {
  if (platform) return { use_platform_key: true };
  const credentials = preview.connect_method === "api_key";
  const values = (
    credentials ? connectCredentialFormSchema : connectOAuthFormSchema
  ).parse(raw);
  const error = credentials
    ? validateConnectCredentialForm(
        values as typeof raw,
        preview.requires_gateway_url,
      )
    : validateConnectOAuthForm(
        values,
        preview.requires_gateway_url,
        connectNeedsOAuthCredentials(preview),
      );
  if (error) throw new Error(error);
  return { ...values, use_platform_key: false };
}
export type ConnectDeviceChallenge = {
  state: string;
  code: string;
  url: string;
  interval: number;
  status: string;
};
export function connectDeviceChallenge(
  result: CompleteConnectLinkResponse,
  previous?: ConnectDeviceChallenge,
): ConnectDeviceChallenge {
  const state = result.device_state ?? previous?.state;
  const code = result.device_user_code ?? previous?.code;
  const url = result.device_verification_uri ?? previous?.url;
  if (!state || !code || !url)
    throw new Error(
      "The provider did not return a complete authorization code. Get a new code.",
    );
  return {
    state,
    code,
    url,
    interval: Math.max(1, result.device_interval ?? previous?.interval ?? 5),
    status: result.device_status ?? previous?.status ?? "pending",
  };
}
/** Match the web's bounded retry policy; never replay a human approval. */
export function connectPollDelay(
  error: unknown,
  failures: number,
  interval: number,
): number | null {
  if (
    failures >= 3 ||
    (isApiError(error) &&
      error.statusCode >= 400 &&
      error.statusCode < 500 &&
      error.statusCode !== 429)
  )
    return null;
  return Math.max(
    1,
    interval + (isApiError(error) && error.statusCode === 429 ? 5 : 0),
  );
}
