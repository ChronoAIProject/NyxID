import { useEffect, useRef } from "react";
import { getAuthEpoch } from "../../lib/auth/authEpoch";
import { useGroupsLive } from "../groups/useGroupsLive";

/** Metadata invalidations refresh authoritative history; never reconstruct a card from SSE. */
export function useAssistantLiveRefresh(
  active: boolean,
  conversationId: string | undefined,
  paused: boolean,
  refresh: (id: string) => Promise<unknown>,
) {
  const epoch = getAuthEpoch();
  const latest = useRef({ active, conversationId, paused, refresh, epoch });
  latest.current = { active, conversationId, paused, refresh, epoch };
  const timer = useRef<ReturnType<typeof setTimeout> | undefined>(undefined);
  const busy = useRef(false);
  const available = () =>
    latest.current.active && latest.current.epoch === getAuthEpoch();
  const live = useGroupsLive(epoch, active, available, (event) => {
    if (!available() || latest.current.paused) return;
    if (
      event.type !== "ready" &&
      event.type !== "resync" &&
      !(
        event.type === "conversation" &&
        event.id === latest.current.conversationId
      )
    )
      return;
    if (timer.current) return;
    function flush() {
      timer.current = undefined;
      const now = latest.current;
      if (!available() || now.paused || !now.conversationId) return;
      if (busy.current) {
        timer.current = setTimeout(flush, 150);
        return;
      }
      busy.current = true;
      void now
        .refresh(now.conversationId)
        .catch(() => undefined)
        .finally(() => {
          busy.current = false;
        });
    }
    timer.current = setTimeout(flush, 150);
  });
  useEffect(
    () => () => {
      clearTimeout(timer.current);
      timer.current = undefined;
    },
    [active, conversationId, epoch],
  );
  useEffect(() => {
    if (!active || !conversationId || live || paused) return;
    const timer = setInterval(() => {
      if (available() && !busy.current) {
        busy.current = true;
        void latest.current
          .refresh(conversationId)
          .catch(() => undefined)
          .finally(() => {
            busy.current = false;
          });
      }
    }, 15000);
    return () => clearInterval(timer);
  }, [active, conversationId, live, paused]);
}
