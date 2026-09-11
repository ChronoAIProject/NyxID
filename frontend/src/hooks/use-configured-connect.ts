import { useEffect, useRef, useState } from "react";
import { useQueryClient } from "@tanstack/react-query";
import {
  useCancelConnectLink,
  useConnectLinkStatus,
  useCreateConnectLink,
} from "@/hooks/use-connect-links";
import {
  removeConfiguredConnectAttempt,
  restoreConfiguredConnectAttempt,
  saveConfiguredConnectAttempt,
} from "@/lib/configured-connect-attempt";
import type { ConfiguredConnectAttempt } from "@/schemas/connect-links";

export function useConfiguredConnect(
  userId: string,
  serviceSlug: string,
  context: string,
) {
  const create = useCreateConnectLink();
  const cancel = useCancelConnectLink();
  const queryClient = useQueryClient();
  const [returnedIds, setReturnedIds] = useState(() =>
    new URLSearchParams(window.location.search).getAll("connect_link_id"),
  );
  const [storageError, setStorageError] = useState<string | null>(null);
  const [request, setRequest] = useState<ConfiguredConnectAttempt | null>(
    () => {
      try {
        return restoreConfiguredConnectAttempt(
          userId,
          serviceSlug,
          context,
          returnedIds,
        );
      } catch {
        return null;
      }
    },
  );
  const status = useConnectLinkStatus(request?.id ?? "", request !== null);
  const unmatchedReturn =
    returnedIds.length > 1 ||
    (returnedIds.length === 1 && returnedIds[0] !== request?.id);
  const mismatchedStatus = Boolean(
    status.data &&
    (status.data.id !== request?.id ||
      status.data.service_slug !== serviceSlug ||
      (status.data.status === "completed" && !status.data.connected_service)),
  );
  const verifiedStatus = mismatchedStatus ? undefined : status.data;
  const terminal =
    verifiedStatus !== undefined && verifiedStatus.status !== "pending";
  const completed =
    !unmatchedReturn &&
    Boolean(request?.callback_url) &&
    verifiedStatus?.status === "completed";
  const invalidatedId = useRef<string | null>(null);

  useEffect(() => {
    if (!completed || !request || invalidatedId.current === request.id) return;
    invalidatedId.current = request.id;
    void queryClient.invalidateQueries({ queryKey: ["keys"] });
  }, [completed, queryClient, request]);

  function clearReturnQuery() {
    const url = new URL(window.location.href);
    url.searchParams.delete("connect_link_id");
    url.searchParams.delete("status");
    window.history.replaceState(null, "", url);
    setReturnedIds([]);
  }

  function prepare(returnPage: string) {
    setStorageError(null);
    create.mutate(
      { service_slug: serviceSlug, return_page: returnPage, expires_in: 900 },
      {
        onSuccess(created) {
          const attempt = {
            ...created,
            service_slug: serviceSlug,
            context,
            return_page: returnPage,
          };
          setRequest(attempt);
          clearReturnQuery();
          try {
            saveConfiguredConnectAttempt(userId, attempt);
          } catch {
            setStorageError(
              "This browser cannot save the connection request. Enable session storage or cancel this request before retrying.",
            );
          }
        },
      },
    );
  }

  function finish() {
    if (!request) return;
    try {
      removeConfiguredConnectAttempt(userId, request.id);
    } catch {
      setStorageError(
        "The saved request could not be cleared. Enable session storage and retry.",
      );
      return;
    }
    clearReturnQuery();
    setStorageError(null);
    setRequest(null);
    create.reset();
    cancel.reset();
  }

  function cancelRequest() {
    if (request)
      cancel.mutate(request.id, {
        onSuccess: () => {
          void status.refetch();
        },
      });
  }

  return {
    request,
    status,
    verifiedStatus,
    unmatchedReturn,
    mismatchedStatus,
    terminal,
    completed,
    canOpen:
      Boolean(request?.callback_url) &&
      !terminal &&
      !storageError &&
      !mismatchedStatus,
    preparing: create.isPending,
    cancelling: cancel.isPending,
    error:
      storageError ??
      create.error?.message ??
      cancel.error?.message ??
      status.error?.message,
    prepare,
    finish,
    cancelRequest,
  };
}
