import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { api } from "@/lib/api-client";
import type { SavedLogin, SavedLoginInput } from "@/schemas/saved-logins";

export function useSavedLogins(owner: string | null = null) {
  return useQuery({
    queryKey: ["saved-logins", owner],
    queryFn: () =>
      api.get<SavedLogin[]>(
        `/saved-logins${owner === "all" ? "?available=true" : owner ? `?owner_id=${encodeURIComponent(owner)}` : ""}`,
      ),
  });
}
// Secrets are sent directly; they are never mutation variables retained in Query.
export async function saveLogin(
  input: SavedLoginInput,
  id?: string,
  owner?: string | null,
) {
  const body = {
    ...input,
    password: input.password || null,
    totp_secret: input.totp_secret || null,
  };
  try {
    return id
      ? await api.put<SavedLogin>(`/saved-logins/${id}`, body)
      : await api.post<SavedLogin>(
          `/saved-logins${owner ? `?owner_id=${encodeURIComponent(owner)}` : ""}`,
          body,
        );
  } finally {
    body.username = "";
    body.password = null;
    body.totp_secret = null;
  }
}
export function useDeleteSavedLogin() {
  const query = useQueryClient();
  return useMutation({
    mutationFn: (id: string) => api.delete(`/saved-logins/${id}`),
    onSuccess: () => query.invalidateQueries({ queryKey: ["saved-logins"] }),
  });
}
