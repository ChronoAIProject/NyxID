import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { api } from "@/lib/api-client";
import {
  completeConnectLinkResponseSchema,
  createConnectLinkResponseSchema,
  connectLinkPreviewSchema,
  connectLinkStatusResponseSchema,
  type CompleteConnectLinkInput,
  type CompleteConnectLinkResponse,
  type ConnectLinkPreview,
  type ConnectLinkStatusResponse,
} from "@/schemas/connect-links";

export function connectLinkStorageKey(id: string): string {
  return `nyxid:connect-link:${id}`;
}

export function compactCompleteConnectLinkInput(
  values: CompleteConnectLinkInput | undefined,
): CompleteConnectLinkInput | undefined {
  if (!values) return undefined;
  return Object.fromEntries(
    Object.entries(values).filter(
      ([, value]) =>
        typeof value === "boolean" ||
        (typeof value === "string" && value.trim().length > 0),
    ),
  );
}

export function usePreviewConnectLink() {
  return useMutation({
    mutationFn: async (token: string): Promise<ConnectLinkPreview> => {
      const response = await api.post<ConnectLinkPreview>(
        "/connect-links/preview",
        { token },
      );
      return connectLinkPreviewSchema.parse(response);
    },
  });
}

export function useCompleteConnectLink() {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: async ({
      token,
      values,
    }: {
      readonly token: string;
      readonly values?: CompleteConnectLinkInput;
    }): Promise<CompleteConnectLinkResponse> => {
      const response = await api.post<CompleteConnectLinkResponse>(
        "/connect-links/complete",
        { token, ...compactCompleteConnectLinkInput(values) },
      );
      return completeConnectLinkResponseSchema.parse(response);
    },
    onSuccess: (result) => {
      if (result.status === "completed") {
        void queryClient.invalidateQueries({ queryKey: ["keys"] });
      }
    },
  });
}

export async function cancelHostedConnectLink(
  token: string,
): Promise<ConnectLinkStatusResponse> {
  const response = await api.post<ConnectLinkStatusResponse>(
    "/connect-links/cancel",
    { token },
  );
  return connectLinkStatusResponseSchema.parse(response);
}

export function useCancelHostedConnectLink() {
  return useMutation({ mutationFn: cancelHostedConnectLink });
}

export function useConnectLinkStatus(id: string, enabled = true) {
  const queryClient = useQueryClient();
  return useQuery({
    queryKey: ["connect-links", id],
    queryFn: async (): Promise<ConnectLinkStatusResponse> => {
      const response = await api.get<ConnectLinkStatusResponse>(
        `/connect-links/${id}`,
      );
      const result = connectLinkStatusResponseSchema.parse(response);
      if (result.status === "completed") {
        void queryClient.invalidateQueries({ queryKey: ["keys"] });
      }
      return result;
    },
    enabled: enabled && id.length > 0,
    refetchInterval: (query) =>
      query.state.data?.status === "pending" ? 1_000 : false,
    refetchOnWindowFocus: true,
  });
}

export interface AssistantConnectLinkInput {
  readonly scopes?: readonly string[];
  readonly serviceSlug: string;
  /**
   * "Connect your own" pins the link to the person's own account. Plain
   * Connect leaves the credential choice to the hosted page, as before.
   */
  readonly personal: boolean;
}

/** Creates a hosted connect link for a user-clicked assistant setup. */
export async function createAssistantConnectLink({ serviceSlug, personal, scopes }: AssistantConnectLinkInput) {
  const response = await api.post("/connect-links", {
    service_slug: serviceSlug,
    ...(personal ? { use_platform_key: false, assistant_personal: true } : {}),
    ...(scopes?.length ? { scopes } : {}),
  });
  return createConnectLinkResponseSchema.parse(response).connect_url;
}

export function useCreateAssistantConnectLink() {
  return useMutation({ mutationFn: createAssistantConnectLink });
}
