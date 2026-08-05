import { useMutation, useQuery } from "@tanstack/react-query";
import { api } from "@/lib/api-client";
import {
  completeConnectLinkResponseSchema,
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
    Object.entries(values).filter(([, value]) => value.trim().length > 0),
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
  });
}

export function useConnectLinkStatus(id: string, enabled = true) {
  return useQuery({
    queryKey: ["connect-links", id],
    queryFn: async (): Promise<ConnectLinkStatusResponse> => {
      const response = await api.get<ConnectLinkStatusResponse>(
        `/connect-links/${id}`,
      );
      return connectLinkStatusResponseSchema.parse(response);
    },
    enabled: enabled && id.length > 0,
    refetchInterval: (query) =>
      query.state.data?.status === "pending" ? 1_000 : false,
    refetchOnWindowFocus: true,
  });
}
