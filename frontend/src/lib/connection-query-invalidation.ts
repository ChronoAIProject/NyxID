import type { QueryClient } from "@tanstack/react-query";
import { PLATFORM_OPERATION_DISCOVERY_QUERY_KEY } from "@/schemas/platform-ops";

export function invalidateConnectionDependents(queryClient: QueryClient): void {
  void queryClient.invalidateQueries({
    queryKey: PLATFORM_OPERATION_DISCOVERY_QUERY_KEY,
  });
}
