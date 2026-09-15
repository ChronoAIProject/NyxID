import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { api } from "@/lib/api-client";
import {
  authorizeContextSchema,
  brandingSchema,
} from "@/schemas/oauth-branding";

export function useAuthorizeContext(ctx: string) {
  return useQuery({
    queryKey: ["authorize-context", ctx],
    queryFn: async ({ signal }) => {
      const response = await fetch(
        `/oauth/authorize-context?ctx=${encodeURIComponent(ctx)}`,
        {
          signal,
          credentials: "include",
          cache: "no-store",
          referrerPolicy: "no-referrer",
        },
      );
      if (!response.ok)
        throw new Error(
          "This sign-in request is unavailable or has expired. Restart sign-in from the app.",
        );
      return authorizeContextSchema.parse(await response.json());
    },
    retry: false,
    staleTime: Infinity,
    gcTime: 0,
    refetchOnWindowFocus: false,
    refetchOnReconnect: false,
  });
}

export function useUploadAppLogo(clientId: string) {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: async (file: File) => {
      if (file.size > 256 * 1024)
        throw new Error("Choose an image no larger than 256 KiB.");
      const body = new FormData();
      body.append("logo", file);
      const response = await fetch(
        `/api/v1/developer/oauth-clients/${encodeURIComponent(clientId)}/branding/logo`,
        {
          method: "POST",
          credentials: "include",
          body,
        },
      );
      if (!response.ok)
        throw new Error(
          "Logo upload failed. Use a PNG or WebP image, at most 256 KiB and 512 by 512 pixels.",
        );
      return brandingSchema.parse(await response.json());
    },
    onSuccess: () => {
      void queryClient.invalidateQueries({
        queryKey: ["developer", "oauth-clients"],
      });
    },
  });
}

export function useVerifyAppBranding(clientId: string) {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: async (body: {
      branding_revision: number;
      verified: boolean;
    }) =>
      brandingSchema.parse(
        await api.post(
          `/admin/oauth-clients/${encodeURIComponent(clientId)}/branding/verify`,
          body,
        ),
      ),
    onSuccess: () => {
      void queryClient.invalidateQueries({
        queryKey: ["admin", "oauth-clients"],
      });
    },
  });
}
