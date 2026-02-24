import { useEffect, useMemo, useState } from "react";
import { useNyxID } from "@nyxid/oauth-react";
import type { LoginRedirectOptions } from "@nyxid/oauth-core";

type PromptValue = "" | "none" | "login" | "consent" | "login consent";

const PROMPT_OPTIONS: { value: PromptValue; label: string }[] = [
  { value: "", label: "Default (skip if consented)" },
  { value: "consent", label: "Force consent" },
  { value: "login", label: "Force re-login" },
  { value: "login consent", label: "Force login + consent" },
  { value: "none", label: "Silent (no UI)" },
];

export function App() {
  const {
    tokens,
    isAuthenticated,
    loginWithRedirect,
    handleRedirectCallback,
    clearSession,
    getUserInfo,
  } = useNyxID();

  const [prompt, setPrompt] = useState<PromptValue>("");
  const [callbackState, setCallbackState] = useState<
    "idle" | "loading" | "success" | "error"
  >("idle");
  const [callbackError, setCallbackError] = useState("");
  const [userInfo, setUserInfo] = useState<object | null>(null);
  const [userInfoError, setUserInfoError] = useState("");

  const isCallbackPath = useMemo(
    () => window.location.pathname === "/auth/callback",
    [],
  );

  useEffect(() => {
    if (!isCallbackPath) return;

    setCallbackState("loading");
    void handleRedirectCallback(window.location.href)
      .then(() => {
        setCallbackState("success");
        // Keep the browser URL clean after successful callback handling.
        window.history.replaceState({}, document.title, "/");
      })
      .catch((error: unknown) => {
        setCallbackState("error");
        setCallbackError(error instanceof Error ? error.message : "Callback failed");
      });
  }, [handleRedirectCallback, isCallbackPath]);

  async function loadUserInfo() {
    setUserInfoError("");
    try {
      const profile = await getUserInfo();
      setUserInfo(profile);
    } catch (error: unknown) {
      setUserInfoError(error instanceof Error ? error.message : "Failed to load userinfo");
    }
  }

  function logout() {
    clearSession();
    setUserInfo(null);
    setUserInfoError("");
    setCallbackState("idle");
    setCallbackError("");
  }

  return (
    <main className="page">
      <section className="card">
        <h1>NyxID OAuth React Demo</h1>
        <p className="muted">
          Demo flow: redirect login, callback token exchange, userinfo fetch, local
          session clear.
        </p>

        {isCallbackPath && (
          <div className="status">
            <strong>Callback status:</strong>{" "}
            {callbackState === "loading" && "Exchanging authorization code..."}
            {callbackState === "success" && "Success. Redirecting to app view."}
            {callbackState === "error" && `Failed: ${callbackError}`}
          </div>
        )}

        <div className="row">
          <select
            value={prompt}
            onChange={(e) => setPrompt(e.target.value as PromptValue)}
          >
            {PROMPT_OPTIONS.map((o) => (
              <option key={o.value} value={o.value}>{o.label}</option>
            ))}
          </select>
          <button
            type="button"
            onClick={() => {
              const opts: LoginRedirectOptions = prompt ? { prompt: prompt as "none" | "consent" | "login" } : {};
              void loginWithRedirect(opts);
            }}
          >
            Login with NyxID
          </button>
          <button type="button" onClick={() => void loadUserInfo()} disabled={!isAuthenticated}>
            Fetch UserInfo
          </button>
          <button type="button" onClick={logout}>
            Clear Session
          </button>
        </div>

        <div className="panel">
          <h2>Auth State</h2>
          <pre>{JSON.stringify({ isAuthenticated }, null, 2)}</pre>
        </div>

        <div className="panel">
          <h2>Token Set</h2>
          <pre>{JSON.stringify(tokens, null, 2)}</pre>
        </div>

        <div className="panel">
          <h2>UserInfo</h2>
          {userInfoError ? <p className="error">{userInfoError}</p> : null}
          <pre>{JSON.stringify(userInfo, null, 2)}</pre>
        </div>
      </section>
    </main>
  );
}
