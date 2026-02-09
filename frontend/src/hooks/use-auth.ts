import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query"
import { api } from "@/lib/api-client"
import type {
  User,
  RegisterResponse,
  MfaSetupResponse,
  MfaVerifyRequest,
} from "@/types/api"
import { useAuthStore } from "@/stores/auth-store"

interface LoginResult {
  readonly mfaRequired: boolean
}

export function useUser() {
  const setUser = useAuthStore((s) => s.setUser)

  return useQuery({
    queryKey: ["user", "me"],
    queryFn: async () => {
      const user = await api.get<User>("/users/me")
      setUser(user)
      return user
    },
    retry: false,
    staleTime: 5 * 60 * 1000,
  })
}

export function useLogin() {
  const queryClient = useQueryClient()
  const login = useAuthStore((s) => s.login)

  return useMutation({
    mutationFn: async (credentials: {
      email: string
      password: string
    }): Promise<LoginResult> => {
      return login(credentials.email, credentials.password)
    },
    onSuccess: (data) => {
      if (!data.mfaRequired) {
        void queryClient.invalidateQueries({ queryKey: ["user"] })
      }
    },
  })
}

export function useRegister() {
  return useMutation({
    mutationFn: async (credentials: {
      email: string
      password: string
      name: string
    }): Promise<RegisterResponse> => {
      return api.post<RegisterResponse>("/auth/register", credentials)
    },
  })
}

export function useLogout() {
  const queryClient = useQueryClient()
  const logout = useAuthStore((s) => s.logout)

  return useMutation({
    mutationFn: async (): Promise<void> => {
      await logout()
    },
    onSuccess: () => {
      queryClient.clear()
    },
  })
}

export function useMfaSetup() {
  return useMutation({
    mutationFn: async (): Promise<MfaSetupResponse> => {
      return api.post<MfaSetupResponse>("/auth/mfa/setup")
    },
  })
}

export function useMfaVerify() {
  const queryClient = useQueryClient()
  const clearMfaState = useAuthStore((s) => s.clearMfaState)

  return useMutation({
    mutationFn: async (data: MfaVerifyRequest): Promise<void> => {
      await api.post<void>("/auth/mfa/verify", data)
    },
    onSuccess: () => {
      clearMfaState()
      void queryClient.invalidateQueries({ queryKey: ["user"] })
    },
  })
}

export function useMfaDisable() {
  const queryClient = useQueryClient()

  return useMutation({
    mutationFn: async (password: string): Promise<void> => {
      return api.post<void>("/auth/mfa/disable", { password })
    },
    onSuccess: () => {
      void queryClient.invalidateQueries({ queryKey: ["user"] })
    },
  })
}
