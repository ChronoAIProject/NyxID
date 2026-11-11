import { useCallback, useEffect, useRef, useState } from "react";
import { ApiError } from "@/lib/api-client";
import { MfaStepUpForm } from "@/components/auth/mfa-step-up";
import { useAuthStore } from "@/stores/auth-store";

function isMfaStepUpRequired(error: unknown): error is ApiError {
  return (
    error instanceof ApiError &&
    error.status === 403 &&
    error.errorCode === 2003
  );
}

/** Each approval surface owns its pending action. Changing account/request
 * clears the prompt; verification never approves a different request. */
export function useMfaStepUp(scope: string, onCancel?: () => void) {
  const accountId = useAuthStore.getState().user?.id;
  const [pending, setPending] = useState<{
    retry: () => Promise<void>;
    message: string;
    scope: string;
    accountId: string | undefined;
  } | null>(null);
  const liveScope = useRef<string | null>(scope);
  useEffect(() => {
    liveScope.current = scope;
    return () => {
      liveScope.current = null;
    };
  }, [scope]);
  if (
    pending &&
    (pending.scope !== scope || pending.accountId !== accountId)
  )
    setPending(null);
  const handle = useCallback(
    (error: unknown, retry: () => Promise<void>): boolean => {
      if (
        !isMfaStepUpRequired(error) ||
        liveScope.current !== scope ||
        useAuthStore.getState().user?.id !== accountId
      )
        return false;
      setPending({ retry, message: error.message, scope, accountId });
      return true;
    },
    [scope, accountId],
  );
  const prompt =
    pending?.scope === scope && pending.accountId === accountId ? (
      <MfaStepUpForm
        message={pending.message}
        onCancel={() => {
          setPending(null);
          onCancel?.();
        }}
        onVerified={async () => {
          setPending((current) => (current === pending ? null : current));
          if (
            liveScope.current !== pending.scope ||
            useAuthStore.getState().user?.id !== pending.accountId
          )
            return;
          await pending.retry();
        }}
      />
    ) : null;
  return { handle, prompt };
}
