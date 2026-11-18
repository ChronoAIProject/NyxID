import { useEffect, useRef, useState } from "react";
import {
  captureAccountOperation,
  getAssistantIdentityRevision,
  getAssistantIdentityUserId,
  isAccountEvidencePending,
  subscribeAccountEvidence,
  subscribeAssistantIdentity,
} from "@/lib/assistant/identity";

type Owner = Pick<ReturnType<typeof captureAccountOperation>, "userId" | "revision">;

/** Accepted effects belong to their initiating owner, independently of dispatch epochs. */
export function useAccountAcceptedResult<T>(onConfirmed: (result: T) => void) {
  const pending = useRef<{ owner: Owner; result: T } | null>(null);
  const [accepted, setAccepted] = useState(false);
  function flush() {
    const entry = pending.current;
    if (!entry) return;
    if (entry.owner.revision !== getAssistantIdentityRevision() ||
        entry.owner.userId !== getAssistantIdentityUserId()) {
      pending.current = null;
      return;
    }
    if (isAccountEvidencePending()) return;
    pending.current = null;
    setAccepted(false);
    onConfirmed(entry.result);
  }
  useEffect(() => {
    const stopEvidence = subscribeAccountEvidence(flush);
    const stopIdentity = subscribeAssistantIdentity(flush);
    return () => { stopEvidence(); stopIdentity(); };
  });
  useEffect(() => () => { pending.current = null; }, []);
  return {
    accepted,
    deliver(owner: Owner, result: T) {
      setAccepted(true);
      pending.current = { owner, result };
      flush();
    },
  };
}
