import { useCallback } from "react";
import { useNavigate, useRouterState } from "@tanstack/react-router";
export function useCrmLocation() {
  const search = useRouterState({ select: (state) => state.location.search });
  const navigate = useNavigate();
  const params = new URLSearchParams(
    Object.entries(search).flatMap(([key, value]) =>
      typeof value === "string" ? [[key, value]] : [],
    ),
  );
  const set = useCallback(
    (
      changes: Readonly<
        Partial<
          Record<
            "owner" | "record" | "view" | "object" | "surface",
            string | undefined
          >
        >
      >,
    ) => {
      void navigate({
        to: "/assistant/crm",
        search: (previous) => ({
          ...previous,
          owner: previous.owner,
          record: previous.record,
          view: previous.view,
          object: previous.object,
          surface: previous.surface,
          ...changes,
        }),
      });
    },
    [navigate],
  );
  return {
    get: (key: string) => params.get(key),
    has: (key: string) => params.has(key),
    set,
  };
}
