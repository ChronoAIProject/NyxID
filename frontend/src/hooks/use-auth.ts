import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { useRef, useState } from "react";
import { api, ApiError } from "@/lib/api-client";
import type {
  User,
  RegisterResponse,
  MfaSetupResponse,
  MfaVerifyRequest,
} from "@/types/api";
import { useAuthStore } from "@/stores/auth-store";

interface LoginResult {
  readonly mfaRequired: boolean;
}

export function useUser() {
  const setUser = useAuthStore((s) => s.setUser);

  return useQuery({
    queryKey: ["user", "me"],
    queryFn: async () => {
      const user = await api.get<User>("/users/me");
      setUser(user);
      return user;
    },
    retry: false,
    staleTime: 5 * 60 * 1000,
    // Server-resolved capabilities (feature flags) can change out from under
    // this session — an admin enabling a flag for the user's org must reach
    // the sidebar without a hard reload. Visible tabs only (default).
    refetchInterval: 60 * 1000,
  });
}

export function useLogin() {
  const queryClient = useQueryClient();
  const login = useAuthStore((s) => s.login);

  return useMutation({
    mutationFn: async (credentials: {
      email: string;
      password: string;
    }): Promise<LoginResult> => {
      return login(credentials.email, credentials.password);
    },
    onSuccess: (data) => {
      if (!data.mfaRequired) {
        void queryClient.invalidateQueries({ queryKey: ["user"] });
      }
    },
  });
}

export function useRegister() {
  return useMutation({
    mutationFn: async (credentials: {
      email: string;
      password: string;
      display_name: string;
      invite_code: string;
    }): Promise<RegisterResponse> => {
      return api.post<RegisterResponse>("/auth/register", credentials);
    },
  });
}

export function useLogout() {
  const queryClient = useQueryClient();
  const logout = useAuthStore((s) => s.logout);

  return useMutation({
    mutationFn: async (): Promise<void> => {
      await logout();
    },
    onSuccess: () => {
      queryClient.clear();
    },
  });
}

export function useMfaSetup() {
  return useMutation({
    mutationFn: async (): Promise<MfaSetupResponse> => {
      return api.post<MfaSetupResponse>("/auth/mfa/setup");
    },
  });
}

export function useMfaVerify() {
  const queryClient = useQueryClient();
  const clearMfaState = useAuthStore((s) => s.clearMfaState);
  const checkAuth = useAuthStore((s) => s.checkAuth);
  const attempt = useRef<{ token: string; needsSessionCheck: boolean } | null>(null);
  const [sessionConfirmationPending, setSessionConfirmationPending] = useState(false);

  const mutation = useMutation({
    scope: { id: "mfa-login" },
    retry: false,
    mutationFn: async (data: MfaVerifyRequest): Promise<void> => {
      if (attempt.current?.token !== data.mfa_token) {
        attempt.current = { token: data.mfa_token, needsSessionCheck: false };
      }
      if (!attempt.current.needsSessionCheck) {
        try {
          await api.post<void>("/auth/mfa/verify", { ...data, client: "web" });
        } catch (error) {
          // A definite client rejection leaves the challenge available for a
          // corrected code. Network/5xx uncertainty can follow consumption of
          // the one-shot token: check the cookie instead of replaying the POST.
          if (error instanceof ApiError && error.status < 500) throw error;
        }
        attempt.current.needsSessionCheck = true;
        setSessionConfirmationPending(true);
      }
      try {
        await checkAuth({ requireSession: true });
      } catch (error) {
        if (error instanceof ApiError && error.status === 401) {
          // The consumed/uncertain challenge cannot recover this session.
          // Return to the existing login flow to obtain a fresh challenge.
          clearMfaState();
        }
        throw new Error("Could not confirm your session. Retry the session check, or return to login.");
      }
    },
    onSuccess: () => {
      clearMfaState();
      void queryClient.invalidateQueries({ queryKey: ["user"] });
    },
  });
  return { ...mutation, sessionConfirmationPending };
}

export function useMfaDisable() {
  const queryClient = useQueryClient();

  return useMutation({
    mutationFn: async (password: string): Promise<void> => {
      return api.post<void>("/auth/mfa/disable", { password });
    },
    onSuccess: () => {
      void queryClient.invalidateQueries({ queryKey: ["user"] });
    },
  });
}

/**
 * Revoke one of the caller's own sessions by id. If the revoked session
 * happens to be the current one, the next API request from this tab will
 * 401 and the auth interceptor bounces the user back to /login — no
 * special-casing needed here, this just refreshes the sessions list.
 *
 * Wave B item B.3.
 */
export function useRevokeSession() {
  const queryClient = useQueryClient();

  return useMutation({
    mutationFn: async (sessionId: string): Promise<void> => {
      return api.delete<void>(`/sessions/${sessionId}`);
    },
    onSuccess: () => {
      void queryClient.invalidateQueries({ queryKey: ["sessions"] });
    },
  });
}
