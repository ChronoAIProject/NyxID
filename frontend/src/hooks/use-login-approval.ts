import { useCallback, useEffect, useRef, useState } from "react";
import { apiClient, ApiError } from "@/lib/api-client";
import {
  approvalIdentitySchema,
  type ApprovalIdentity,
} from "@/schemas/login-approval";
import {
  agentKeyOptionsSchema,
  type AgentKeyApprove,
} from "@/schemas/agent-key-login";
import {
  loginCatalogSchema,
  type LoginInventory,
} from "@/lib/login-permissions";
import { userCodeSchema } from "@/schemas/auth-device";
import { useAuthStore } from "@/stores/auth-store";
import type { LoginFlow } from "./use-agent-key-login";

const storageKey = (flow: LoginFlow, code: string) =>
  `nyxid-approval:${flow}:${code}`;
export function approvalQuery(flow: LoginFlow, query: string): string {
  const params = new URLSearchParams(query);
  const code = userCodeSchema.safeParse(params.get("user_code"));
  if (!code.success || [...params.keys()].some((k) => k !== "user_code"))
    return query;
  try {
    const saved = JSON.parse(
      sessionStorage.getItem(storageKey(flow, code.data)) ?? "null",
    );
    if (
      saved &&
      Date.parse(saved.expires_at) > Date.now() &&
      typeof saved.query === "string" &&
      userCodeSchema.safeParse(
        new URLSearchParams(saved.query).get("user_code"),
      ).data === code.data
    )
      return saved.query;
  } catch {
    /* The current link remains usable without saved hints. */
  }
  return query;
}

export function useLoginApproval(flow: LoginFlow, code: string, query: string) {
  const [identity, setIdentity] = useState<ApprovalIdentity | null>(null);
  const [restoring, setRestoring] = useState(false);
  const current = useRef(identity);
  current.current = identity;
  const key = storageKey(flow, code);
  const call = (
    path: string,
    body?: unknown,
    method = body === undefined ? "GET" : "POST",
  ) =>
    apiClient<unknown>(`/auth/approval${path}`, {
      method,
      body,
      preserveSessionOn401: true,
    });
  const parse = useCallback(
    (value: unknown) => {
      const result = approvalIdentitySchema.parse(value);
      if (result.flow !== flow || result.user_code !== code)
        throw Error("This verification belongs to a different request.");
      return result;
    },
    [flow, code],
  );
  useEffect(() => {
    let alive = true;
    let saved: { id: string; expires_at: string } | null = null;
    try {
      saved = JSON.parse(sessionStorage.getItem(key) ?? "null");
    } catch {
      /* No resumable identity. */
    }
    if (
      saved &&
      /^[a-f0-9-]{36}$/.test(saved.id) &&
      Date.parse(saved.expires_at) > Date.now()
    ) {
      setRestoring(true);
      void apiClient<unknown>(`/auth/approval/${saved.id}`, {
        preserveSessionOn401: true,
      })
        .then((value) => {
          if (alive) setIdentity(parse(value));
        })
        .catch(() => {
          if (alive) {
            setIdentity(null);
            try {
              sessionStorage.removeItem(key);
            } catch {
              /* Storage can be disabled while verification is in progress. */
            }
          }
        })
        .finally(() => {
          if (alive) setRestoring(false);
        });
    }
    return () => {
      alive = false;
    };
  }, [key, parse]);
  async function begin(keep: boolean) {
    if (current.current) return current.current;
    const next = parse(
      await call("", { flow, user_code: code, keep_signed_in: keep }),
    );
    current.current = next;
    setIdentity(next);
    try {
      sessionStorage.setItem(
        key,
        JSON.stringify({ ...next, user: undefined, query }),
      );
    } catch {
      /* Password verification still works in memory. */
    }
    return next;
  }
  async function authenticate(
    keep: boolean,
    password: { email: string; password: string },
  ) {
    const next = await begin(keep);
    const result = parse(await call(`/${next.id}/password`, password));
    current.current = result;
    setIdentity(result);
    if (result.verified && result.keep_signed_in)
      await useAuthStore.getState().checkAuth({ ephemeral: true });
    return result;
  }
  async function beginSocial(keep: boolean) {
    const next = await begin(keep);
    try {
      if (JSON.parse(sessionStorage.getItem(key) ?? "null")?.id === next.id)
        return next;
    } catch {
      /* Redirecting requires a resumable context in this tab. */
    }
    throw Error(
      "Allow site storage to continue with a provider, or verify with your password.",
    );
  }
  async function mfa(code: string) {
    if (!current.current) throw Error("Verify your identity again.");
    const result = parse(await call(`/${current.current.id}/mfa`, { code }));
    current.current = result;
    setIdentity(result);
    if (result.verified && result.keep_signed_in)
      await useAuthStore.getState().checkAuth({ ephemeral: true });
    return result;
  }
  async function inventory(): Promise<LoginInventory> {
    if (!current.current?.verified) throw Error("Verify your identity again.");
    const result = (await call(`/${current.current.id}/inventory`)) as {
      options: unknown;
      catalog: unknown;
    };
    const options = agentKeyOptionsSchema.parse(result.options);
    return {
      options,
      connections: options.connections ?? [],
      catalog: loginCatalogSchema.parse(result.catalog).entries,
    };
  }
  async function decide(
    body: Omit<AgentKeyApprove, "user_code"> | undefined,
    deny = false,
  ) {
    if (!current.current?.verified) throw Error("Verify your identity again.");
    await call(
      `/${current.current.id}/${deny ? "deny" : "approve"}`,
      deny ? undefined : (body ?? {}),
      "POST",
    );
    forget();
  }
  const forget = useCallback(() => {
    try {
      sessionStorage.removeItem(key);
    } catch {
      /* Optional persistence. */
    }
  }, [key]);
  async function reset() {
    if (current.current) {
      try {
        await call(`/${current.current.id}`, undefined, "DELETE");
      } catch (error) {
        if (!(error instanceof ApiError && error.status === 401)) throw error;
      }
    }
    current.current = null;
    setIdentity(null);
    forget();
  }
  return {
    identity,
    restoring,
    begin,
    beginSocial,
    authenticate,
    mfa,
    inventory,
    decide,
    reset,
    forget,
    current,
  };
}
