import { useEffect, useRef, useState, useSyncExternalStore } from "react";
import { WebsiteRequestError } from "./public-feature-adapters";
const noSubscribe = () => () => {};
/** Same inert-SSR boundary as Contact; requests exist only after a real submit. */
export function usePublicFeatureRequest() {
  const interactive = useSyncExternalStore(
    noSubscribe,
    () => true,
    () => false,
  );
  const active = useRef<AbortController | null>(null);
  const [pending, setPending] = useState(false),
    [failed, setFailed] = useState(false);
  useEffect(
    () => () => {
      active.current?.abort();
    },
    [],
  );
  async function run<T>(
    operation: (signal: AbortSignal) => Promise<T>,
  ): Promise<T | undefined> {
    active.current?.abort();
    const request = new AbortController();
    active.current = request;
    setPending(true);
    setFailed(false);
    try {
      const result = await operation(request.signal);
      return request.signal.aborted ? undefined : result;
    } catch (error) {
      if (!request.signal.aborted) {
        setFailed(true);
        console.warn(
          "[website-feature] Request failed",
          error instanceof WebsiteRequestError
            ? error.traceId
            : crypto.randomUUID(),
        );
      }
      return undefined;
    } finally {
      if (!request.signal.aborted) setPending(false);
    }
  }
  return { interactive, pending, failed, run };
}
