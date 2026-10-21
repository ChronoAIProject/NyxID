/** Keep late responses from refreshing or invalidating a replacement session. */
export async function guardedJsonTransport<T>(options: {
  send: () => Promise<Response>;
  read: (response: Response) => Promise<unknown>;
  guard: () => void;
  refresh: () => Promise<boolean>;
  invalidate: () => Promise<void> | void;
  failure: (payload: unknown, status: number) => Error;
  requiresAuth: boolean;
  retry: boolean;
}): Promise<T> {
  const receive = async () => {
    options.guard();
    const response = await options.send();
    options.guard();
    const payload = await options.read(response);
    options.guard();
    return { response, payload };
  };
  let { response, payload } = await receive();
  if (response.status === 401 && options.requiresAuth && options.retry) {
    options.guard();
    const refreshed = await options.refresh();
    options.guard();
    if (refreshed) ({ response, payload } = await receive());
  }
  options.guard();
  if (!response.ok) {
    if (response.status === 401 && options.requiresAuth) {
      options.guard();
      await options.invalidate();
      options.guard();
    }
    throw options.failure(payload, response.status);
  }
  return payload as T;
}
