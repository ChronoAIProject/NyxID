import { useQuery } from "@tanstack/react-query";
import { assistantJson } from "@/lib/assistant/assistant-http";
import { useFeature } from "@/hooks/use-feature-flag";
import { FEATURE_FLAG } from "@/lib/feature-flags";
import { useAuthStore } from "@/stores/auth-store";
import { z } from "zod";
import {
  useApprovalRequests,
  useNotificationSettings,
} from "@/hooks/use-approvals";
import {
  toAssistantApprovalEntry,
  type AssistantApprovalEntry,
} from "@/lib/assistant/approvals";

const PENDING_APPROVALS_PAGE_SIZE = 50;
const APPROVAL_HISTORY_PAGE_SIZE = 50;

function usePendingApprovalRequests() {
  return useApprovalRequests(1, PENDING_APPROVALS_PAGE_SIZE, "pending");
}

export function useAssistantApprovals() {
  const settings = useNotificationSettings();
  const pendingQuery = usePendingApprovalRequests();
  const historyQuery = useApprovalRequests(1, APPROVAL_HISTORY_PAGE_SIZE);
  const grantDurationSec = settings.data
    ? settings.data.grant_expiry_days * 86_400
    : null;
  const pending: AssistantApprovalEntry[] = (
    pendingQuery.data?.requests ?? []
  ).map((request) => toAssistantApprovalEntry(request, grantDurationSec));
  const history: AssistantApprovalEntry[] = (historyQuery.data?.requests ?? [])
    .filter((request) => request.status !== "pending")
    .map((request) => toAssistantApprovalEntry(request, grantDurationSec));

  return {
    pending,
    history,
    isLoading: pendingQuery.isLoading || historyQuery.isLoading,
    isError: pendingQuery.isError || historyQuery.isError,
    refetch: () => {
      void pendingQuery.refetch();
      void historyQuery.refetch();
    },
  };
}

export function useAssistantWorkspaceCounts() {
  const pendingQuery = usePendingApprovalRequests();
  const enabled = useFeature(FEATURE_FLAG.CMS_POC_V1);
  const actor = useAuthStore((state) => state.user?.id);
  const artifacts = useQuery({
    queryKey: ["crm", actor, "artifact-count", "actor", actor],
    queryFn: async () =>
      z
        .object({ count: z.number().int().nonnegative() })
        .parse(await assistantJson("/assistant/crm/views/count")).count,
    enabled: enabled && Boolean(actor),
    retry: false,
    refetchInterval: 30_000,
  });
  return {
    data: {
      artifacts: enabled ? (artifacts.data ?? 0) : 0,
      pendingApprovals: pendingQuery.data?.total ?? 0,
    },
  };
}
