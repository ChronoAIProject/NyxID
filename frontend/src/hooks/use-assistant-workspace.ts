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
  return {
    data: {
      artifacts: 0,
      pendingApprovals: pendingQuery.data?.total ?? 0,
    },
  };
}
