import { useMutation, useQueryClient } from "@tanstack/react-query";
import { api } from "@/lib/api-client";
import {
  approveDeviceResponseSchema,
  type ApproveDeviceRequest,
  type ApproveDeviceResponse,
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
