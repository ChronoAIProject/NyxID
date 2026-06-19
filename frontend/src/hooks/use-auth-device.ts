import { useMutation } from "@tanstack/react-query";
import { api } from "@/lib/api-client";
import {
  approveBodySchema,
  approveResponseSchema,
  previewResponseSchema,
  type ApproveAuthDeviceResponse,
  type PreviewAuthDeviceResponse,
} from "@/schemas/auth-device";

export function usePreviewAuthDevice() {
  return useMutation({
    mutationFn: async (
      userCode: string,
    ): Promise<PreviewAuthDeviceResponse> => {
      const response = await api.post<PreviewAuthDeviceResponse>(
        "/auth/device/preview",
        { user_code: userCode },
      );
      return previewResponseSchema.parse(response);
    },
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
