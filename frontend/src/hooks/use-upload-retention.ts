import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { api } from "@/lib/api-client";
import {
  uploadRetentionResponseSchema,
  type UploadRetentionPolicy,
} from "@/schemas/upload-retention";
const path = "/admin/settings/upload-retention";
const key = ["admin", "upload-retention"] as const;
export function useUploadRetention() {
  return useQuery({
    queryKey: key,
    queryFn: async () =>
      uploadRetentionResponseSchema.parse(await api.get(path)),
    refetchInterval: 5000,
  });
}
export function useUpdateUploadRetention() {
  const client = useQueryClient();
  return useMutation({
    mutationFn: async (policy: UploadRetentionPolicy | null) =>
      uploadRetentionResponseSchema.parse(
        policy === null ? await api.delete(path) : await api.put(path, policy),
      ),
    onSuccess: (data) => {
      client.setQueryData(key, data);
    },
  });
}
