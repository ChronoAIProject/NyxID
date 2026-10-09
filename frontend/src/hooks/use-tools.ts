import { useAuthStore } from "@/stores/auth-store";
import { useQuery } from "@tanstack/react-query";
import { api } from "@/lib/api-client";
import type { ToolOffering } from "@/schemas/tools";
export function useTools() {
  const identity = useAuthStore((state) => state.user?.id);
  return useQuery({
    queryKey: ["tools", identity],
    queryFn: () => api.get<ToolOffering[]>("/tools"),
  });
}
export function useToolTopics() {
  return useQuery({
    queryKey: ["tools", "topics"],
    queryFn: () => api.get<{ slug: string; label: string }[]>("/tools/topics"),
    staleTime: Infinity,
  });
}
