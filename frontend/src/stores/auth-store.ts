import { create } from "zustand";
import type { User, LoginResponse } from "@/types/api";
import { api, ApiError } from "@/lib/api-client";

const MFA_REQUIRED_ERROR_CODE = 2002;

interface LoginResult {
  readonly mfaRequired: boolean;
  readonly response?: LoginResponse;
}

interface AuthState {
  readonly user: User | null;
  readonly isAuthenticated: boolean;
  readonly isLoading: boolean;
  readonly mfaRequired: boolean;
  readonly mfaToken: string | null;
}

interface AuthActions {
  readonly login: (email: string, password: string) => Promise<LoginResult>;
  readonly logout: () => Promise<void>;
  readonly checkAuth: () => Promise<void>;
  readonly setUser: (user: User | null) => void;
  readonly setMfaRequired: (required: boolean, token: string | null) => void;
  readonly clearMfaState: () => void;
}

type AuthStore = AuthState & AuthActions;

export const useAuthStore = create<AuthStore>((set) => ({
  user: null,
  isAuthenticated: false,
  isLoading: true,
  mfaRequired: false,
  mfaToken: null,

  login: async (email: string, password: string): Promise<LoginResult> => {
    try {
      const response = await api.post<LoginResponse>("/auth/login", {
        email,
        password,
        client: "web",
      });

      set({
        isAuthenticated: true,
        mfaRequired: false,
        mfaToken: null,
      });

      return { mfaRequired: false, response };
    } catch (error) {
      if (
        error instanceof ApiError &&
        error.errorCode === MFA_REQUIRED_ERROR_CODE
      ) {
        const sessionToken =
          (error.errorResponse as { session_token?: string }).session_token ??
          null;
        set({
          mfaRequired: true,
          mfaToken: sessionToken,
        });
        return { mfaRequired: true };
      }
      throw error;
    }
  },

  logout: async (): Promise<void> => {
    try {
      await api.post<void>("/auth/logout");
    } finally {
      set({
        user: null,
        isAuthenticated: false,
        mfaRequired: false,
        mfaToken: null,
      });
    }
  },

  checkAuth: async (): Promise<void> => {
    set({ isLoading: true });
    try {
      const user = await api.get<User>("/users/me");
      set({ user, isAuthenticated: true, isLoading: false });
    } catch (error) {
      if (error instanceof ApiError && error.status === 401) {
        set({ user: null, isAuthenticated: false, isLoading: false });
      } else {
        set({ isLoading: false });
      }
    }
  },

  setUser: (user: User | null): void => {
    set({ user, isAuthenticated: user !== null });
  },

  setMfaRequired: (required: boolean, token: string | null): void => {
    set({ mfaRequired: required, mfaToken: token });
  },

  clearMfaState: (): void => {
    set({ mfaRequired: false, mfaToken: null });
  },
}));
