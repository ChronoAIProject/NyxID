import {
  parsePublicAuthConfig,
  type PublicAuthConfig,
} from "../../features/auth/publicAuthConfig";
import { requestJson } from "./http";

const PUBLIC_AUTH_CONFIG_TIMEOUT_MS = 10_000;

function cancelledRequest(): Error {
  return Object.assign(new Error("Loading sign-in options was cancelled."), {
    name: "AbortError",
  });
}

export async function getPublicAuthConfig(
  signal?: AbortSignal,
): Promise<PublicAuthConfig> {
  const boundedController = new AbortController();
  if (signal?.aborted) throw cancelledRequest();

  const forwardAbort = () => boundedController.abort();
  signal?.addEventListener("abort", forwardAbort, { once: true });
  let timedOut = false;
  const timeout = setTimeout(() => {
    timedOut = true;
    boundedController.abort();
  }, PUBLIC_AUTH_CONFIG_TIMEOUT_MS);

  try {
    const response = await requestJson<unknown>("/public/config", {
      requiresAuth: false,
      signal: boundedController.signal,
    });
    if (signal?.aborted) throw cancelledRequest();
    if (timedOut) {
      throw new Error("Loading sign-in options timed out. Please try again.");
    }
    return parsePublicAuthConfig(response);
  } catch {
    if (signal?.aborted) throw cancelledRequest();
    if (timedOut) {
      throw new Error(
        "Loading sign-in options timed out. Check your connection and try again.",
      );
    }
    // API errors and invalid response bodies may contain server details.
    throw new Error(
      "Unable to load sign-in options. Check your connection and try again.",
    );
  } finally {
    clearTimeout(timeout);
    signal?.removeEventListener("abort", forwardAbort);
  }
}
