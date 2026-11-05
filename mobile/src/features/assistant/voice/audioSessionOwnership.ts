let owner: symbol | undefined;

/** Delayed native cleanup may relinquish only the session it acquired. */
export function claimVoiceAudioSession() {
  const token = Symbol("voice audio session");
  owner = token;
  return {
    release(cleanup?: () => void) {
      if (owner !== token) return;
      owner = undefined;
      cleanup?.();
    },
  };
}
