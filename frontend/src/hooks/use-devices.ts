import { useMutation, useQueryClient } from "@tanstack/react-query";
import { api } from "@/lib/api-client";
import {
  approveDeviceResponseSchema,
  onboardDeviceResponseSchema,
  type ApproveDeviceRequest,
  type ApproveDeviceResponse,
  type OnboardDeviceRequest,
  type OnboardDeviceResponse,
} from "@/schemas/devices";

export function useApproveDevice() {
  const queryClient = useQueryClient();

  return useMutation({
    mutationFn: async (
      body: ApproveDeviceRequest,
    ): Promise<ApproveDeviceResponse> => {
      const response = await api.post<ApproveDeviceResponse>(
        "/devices/code/approve",
        body,
      );
      return approveDeviceResponseSchema.parse(response);
    },
    onSuccess: () => {
      void queryClient.invalidateQueries({ queryKey: ["api-keys"] });
      void queryClient.invalidateQueries({ queryKey: ["keys"] });
      void queryClient.invalidateQueries({ queryKey: ["nodes"] });
    },
  });
}

export function useOnboardDevice() {
  const queryClient = useQueryClient();

  return useMutation({
    mutationFn: async (
      body: OnboardDeviceRequest,
    ): Promise<OnboardDeviceResponse> => {
      const response = await api.post<OnboardDeviceResponse>(
        "/devices/onboard",
        body,
      );
      return onboardDeviceResponseSchema.parse(response);
    },
    onSuccess: () => {
      void queryClient.invalidateQueries({ queryKey: ["api-keys"] });
      void queryClient.invalidateQueries({ queryKey: ["keys"] });
      void queryClient.invalidateQueries({ queryKey: ["nodes"] });
    },
  });
}
