import { ApiError } from "@/lib/api-client";
import { useAuthStore } from "@/stores/auth-store";

/** A preparing response leaves the account usable; retry only that response. */
export async function withErasurePreparation<T>(request: () => Promise<T>): Promise<T> {
  const owner = useAuthStore.getState().user?.id;
  for (let attempt = 0; ; attempt += 1) {
    if (useAuthStore.getState().user?.id !== owner) {
      throw new Error("The signed-in account changed. Review account deletion again.");
    }
    try {
      return await request();
    } catch (error) {
      if (!(error instanceof ApiError) || error.status !== 409 || error.errorCode !== 12801) throw error;
      if (attempt >= 9) throw new Error("Account deletion is still being prepared; try again in a minute.");
      await new Promise<void>((resolve) => setTimeout(resolve, Math.min(500 * 2 ** attempt, 5000)));
    }
  }
}
