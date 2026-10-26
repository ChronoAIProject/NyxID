import {
  createContext,
  useCallback,
  useContext,
  useEffect,
  useMemo,
  useState,
  type ReactNode,
} from "react";
import {
  NyxIDClient,
  type LoginRedirectOptions,
  type NyxIDClientConfig,
  type NyxIDTokenSet,
  type OAuthUserInfo,
} from "@nyxids/oauth-core";

interface NyxIDContextValue {
  readonly client: NyxIDClient;
  readonly tokens: NyxIDTokenSet | null;
  readonly isAuthenticated: boolean;
  readonly loginWithRedirect: (options?: LoginRedirectOptions) => Promise<void>;
  readonly handleRedirectCallback: (url?: string) => Promise<NyxIDTokenSet>;
  readonly clearSession: () => void;
  readonly refreshToken: () => Promise<NyxIDTokenSet>;
  readonly getAccessToken: () => Promise<string>;
  readonly getUserInfo: (accessToken?: string) => Promise<OAuthUserInfo>;
}

const NyxIDContext = createContext<NyxIDContextValue | null>(null);

export function createNyxClient(config: NyxIDClientConfig): NyxIDClient {
  return new NyxIDClient(config);
}

export function NyxIDProvider({
  client,
  children,
}: {
  readonly client: NyxIDClient;
  readonly children: ReactNode;
}) {
  const [tokens, setTokens] = useState<NyxIDTokenSet | null>(() =>
    client.getStoredTokens(),
  );

  const loginWithRedirect = useCallback(
    async (options?: LoginRedirectOptions) => {
      await client.loginWithRedirect(options);
    },
    [client],
  );

  const handleRedirectCallback = useCallback(
    async (url?: string) => {
      const nextTokens = await client.handleRedirectCallback(url);
      setTokens(nextTokens);
      return nextTokens;
    },
    [client],
  );

  const clearSession = useCallback(() => {
    client.clearSession();
    setTokens(null);
  }, [client]);

  const refreshToken = useCallback(async () => {
    try {
      return await client.refreshToken();
    } finally {
      setTokens(client.getStoredTokens());
    }
  }, [client]);

  const getAccessToken = useCallback(async () => {
    try {
      return await client.getAccessToken();
    } finally {
      setTokens(client.getStoredTokens());
    }
  }, [client]);

  useEffect(() => {
    setTokens(client.getStoredTokens());
    const sync = () => setTokens(client.getStoredTokens());
    window.addEventListener("storage", sync);
    const tick = () => {
      void client.getAccessToken()
        .then(() => setTokens(client.getStoredTokens()))
        .catch(() => setTokens(client.getStoredTokens()));
    };
    tick();
    const timer = window.setInterval(tick, 30_000);
    return () => {
      window.clearInterval(timer);
      window.removeEventListener("storage", sync);
    };
  }, [client]);

  const getUserInfo = useCallback(
    async (accessToken?: string) => {
      try {
        return await client.getUserInfo(accessToken);
      } finally {
        setTokens(client.getStoredTokens());
      }
    },
    [client],
  );

  const value = useMemo<NyxIDContextValue>(
    () => ({
      client,
      tokens,
      isAuthenticated: Boolean(tokens?.accessToken),
      loginWithRedirect,
      handleRedirectCallback,
      clearSession,
      refreshToken,
      getAccessToken,
      getUserInfo,
    }),
    [
      client,
      tokens,
      loginWithRedirect,
      handleRedirectCallback,
      clearSession,
      refreshToken,
      getAccessToken,
      getUserInfo,
    ],
  );

  return <NyxIDContext.Provider value={value}>{children}</NyxIDContext.Provider>;
}

export function useNyxID(): NyxIDContextValue {
  const context = useContext(NyxIDContext);
  if (!context) {
    throw new Error("useNyxID must be used inside NyxIDProvider");
  }
  return context;
}
