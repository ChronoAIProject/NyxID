import { RefreshRetryableError, type RefreshOutcome } from '../auth/refreshOutcome';
/** Every response is checked against the initiating login. Invalidation is a
 * coordinator compare-and-clear of the exact pair used by the failing send. */
export async function guardedJsonTransport<T>(options: {
  send: () => Promise<Response>;
  read: (response: Response) => Promise<unknown>;
  guard: () => void;
  refresh: () => Promise<RefreshOutcome>;
  invalidate: () => Promise<void> | void;
  failure: (payload: unknown, status: number) => Error;
  requiresAuth: boolean;
  retry: boolean;
}): Promise<T> {
  const receive = async () => {
    options.guard();
    const response = await options.send(); options.guard();
    const payload = await options.read(response); options.guard();
    return { response, payload };
  };
  let { response, payload } = await receive();
  let invalidate = response.status === 401 && !options.retry;
  if (response.status === 401 && options.requiresAuth && options.retry) {
    const outcome = await options.refresh(); options.guard();
    if (outcome.kind === 'transient' || outcome.kind === 'persistence_failed') throw new RefreshRetryableError(outcome);
    if (outcome.kind === 'refreshed' || (outcome.kind === 'superseded' && outcome.current)) {
      ({ response, payload } = await receive());
      invalidate = response.status === 401;
    } else { invalidate = outcome.kind === 'rejected' || outcome.kind === 'no_credentials'; }
  }
  options.guard();
  if (!response.ok) {
    if (invalidate && options.requiresAuth) { options.guard(); await options.invalidate(); }
    throw options.failure(payload, response.status);
  }
  return payload as T;
}
