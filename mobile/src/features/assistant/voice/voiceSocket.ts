import type { VoiceControlSocket } from "./AssistantVoiceClient";
import {
  assistantVoiceStreamPath,
  assistantDictationPath,
  trustedVoiceOrigin,
} from "../../../lib/api/assistantVoiceTransport";

type Session = { accessToken: string; userId?: string };
type SocketConstructor = new (
  url: string,
  protocols: string[] | undefined,
  options: { headers: Record<string, string> },
) => VoiceControlSocket;

/** Credentials stay in native headers and can never reach the media WebView. */
export function createVoiceSocketFactory(input: {
  apiBaseUrl: string;
  origin: string;
  epoch: number;
  getEpoch: () => number;
  refresh: () => Promise<unknown>;
  loadSession: () => Promise<Session | null>;
  Socket: SocketConstructor;
}) {
  const origin = trustedVoiceOrigin(input.origin);
  return async (path: string, signal: AbortSignal) => {
    const current = () => {
      if (signal.aborted || input.getEpoch() !== input.epoch)
        throw Object.assign(new Error("Voice connection cancelled."), {
          name: "AbortError",
        });
    };
    current();
    const match =
      /^\/assistant\/nyxagent\/conversations\/([^/]+)\/voice-sessions\/([^/]+)\/stream$/.exec(
        path,
      );
    const dictation =
      /^\/assistant\/nyxagent\/conversations\/([^/]+)\/dictation$/.exec(path);
    const validStream =
      match?.[1] &&
      match[2] &&
      assistantVoiceStreamPath(match[1], match[2]) === path;
    const validDictation =
      dictation?.[1] && assistantDictationPath(dictation[1]) === path;
    if (!validStream && !validDictation)
      throw new Error("Invalid voice connection.");
    const base = new URL(input.apiBaseUrl);
    if (
      !["https:", "http:"].includes(base.protocol) ||
      base.username ||
      base.password ||
      base.search ||
      base.hash
    )
      throw new Error("Voice requires a configured NyxID server.");
    await input.refresh();
    current();
    const session = await input.loadSession();
    current();
    if (!session?.accessToken) throw new Error("Sign in again to use voice.");
    const url = new URL(
      `${base.pathname.replace(/\/+$/, "")}${path}`,
      base.origin,
    );
    url.protocol = base.protocol === "https:" ? "wss:" : "ws:";
    return new input.Socket(url.href, undefined, {
      headers: {
        Authorization: `Bearer ${session.accessToken}`,
        Origin: origin,
        "X-NyxID-Client": "mobile",
      },
    });
  };
}
