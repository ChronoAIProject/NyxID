import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { api } from "@/lib/api-client";
import {
  type ChainVerificationResponse,
  chainVerificationResponseSchema,
} from "@/schemas/admin-chain-verification";

const CHAIN_VERIFICATION_KEY = ["admin", "chain-verification"] as const;

export function useChainVerification() {
  return useQuery({
    queryKey: CHAIN_VERIFICATION_KEY,
    queryFn: async (): Promise<ChainVerificationResponse> => {
      const raw = await api.get<unknown>("/admin/chain-verification");
      return chainVerificationResponseSchema.parse(raw);
    },
    refetchInterval: 60_000,
  });
}

export function useRunChainVerification() {
  const queryClient = useQueryClient();

  return useMutation({
    mutationFn: async (): Promise<ChainVerificationResponse> => {
      const raw = await api.post<unknown>("/admin/chain-verification/run");
      return chainVerificationResponseSchema.parse(raw);
    },
    onSuccess: (data) => {
      queryClient.setQueryData(CHAIN_VERIFICATION_KEY, data);
    },
  });
}
