import { useMutation, useQuery } from "@tanstack/react-query";
import { api } from "@/lib/api-client";
import {
  approveBodySchema,
  approveResponseSchema,
  previewResponseSchema,
  type ApproveAuthDeviceResponse,
  type PreviewAuthDeviceResponse,
} from "@/schemas/auth-device";

export function usePreviewAuthDevice(userCode: string | null) {
  return useQuery({
    queryKey: ["auth-device-preview", userCode],
    queryFn: async (): Promise<PreviewAuthDeviceResponse | null> => {
      if (!userCode) return null;
      const response = await api.post<PreviewAuthDeviceResponse>(
        "/auth/device/preview",
        { user_code: userCode },
      );
      return previewResponseSchema.parse(response);
    },
    enabled: !!userCode,
    retry: false,
  });
}

export function useApproveAuthDevice() {
  return useMutation({
    mutationFn: async (
      userCode: string,
    ): Promise<ApproveAuthDeviceResponse> => {
      const body = approveBodySchema.parse({ user_code: userCode });
      const response = await api.post<ApproveAuthDeviceResponse>(
        "/auth/device/approve",
        body,
      );
      return approveResponseSchema.parse(response);
    },
  });
}
