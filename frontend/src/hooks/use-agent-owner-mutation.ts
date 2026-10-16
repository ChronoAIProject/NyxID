import { useCallback } from "react";
import { useMutation, type MutateOptions } from "@tanstack/react-query";
import { useAuthStore } from "@/stores/auth-store";

function assertOwner(owner: string | undefined): asserts owner is string {
  if (!owner || useAuthStore.getState().user?.id !== owner) {
    throw new Error("Your account changed. Reload before managing Agent permissions.");
  }
}

/** Bind existing Agent permission writes before asynchronous mutation middleware. */
export function useAgentOwnerMutation<Data, Body>(options: {
  mutationFn: (body: Body, authorityGuard: () => void) => Promise<Data>;
  onSettled: (owner: string) => Promise<unknown>;
}) {
  const owner = useAuthStore((state) => state.user?.id);
  const mutation = useMutation({
    mutationFn: async ({ body, owner: submittedOwner }: { body: Body; owner: string }) => {
      const guard = () => assertOwner(submittedOwner);
      guard();
      const result = await options.mutationFn(body, guard);
      guard();
      return result;
    },
    retry: false,
    onSettled: async (_data, _error, { owner: submittedOwner }) => {
      if (useAuthStore.getState().user?.id === submittedOwner) {
        await options.onSettled(submittedOwner);
      }
    },
  });
  const dispatch = mutation.mutate;
  const dispatchAsync = mutation.mutateAsync;
  const callbacksFor = useCallback((callbacks?: MutateOptions<Data, Error, Body>) => ({
    onSuccess: (data: Data, variables: { body: Body; owner: string }, result: unknown, context: Parameters<NonNullable<MutateOptions<Data, Error, Body>["onSuccess"]>>[3]) => {
      if (useAuthStore.getState().user?.id === variables.owner) callbacks?.onSuccess?.(data, variables.body, result, context);
    },
    onError: (error: Error, variables: { body: Body; owner: string }, result: unknown, context: Parameters<NonNullable<MutateOptions<Data, Error, Body>["onError"]>>[3]) => callbacks?.onError?.(error, variables.body, result, context),
    onSettled: (data: Data | undefined, error: Error | null, variables: { body: Body; owner: string }, result: unknown, context: Parameters<NonNullable<MutateOptions<Data, Error, Body>["onSettled"]>>[4]) => callbacks?.onSettled?.(data, error, variables.body, result, context),
  }), []);
  const mutate = useCallback((body: Body, callbacks?: MutateOptions<Data, Error, Body>) => {
    assertOwner(owner);
    dispatch({ body, owner }, callbacksFor(callbacks));
  }, [dispatch, owner, callbacksFor]);
  const mutateAsync = useCallback(async (body: Body, callbacks?: MutateOptions<Data, Error, Body>) => {
    assertOwner(owner);
    return dispatchAsync({ body, owner }, callbacksFor(callbacks));
  }, [dispatchAsync, owner, callbacksFor]);
  return { ...mutation, variables: mutation.variables?.body, mutate, mutateAsync };
}
