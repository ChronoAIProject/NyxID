const owners = new Set<symbol>();
let deactivation: (() => void) | undefined;

/** Keep native cleanup owed by a finished owner until every audio user stops. */
export function claimVoiceAudioSession() {
  const token = Symbol("voice audio session");
  owners.add(token);
  return {
    release(cleanup?: () => void) {
      if (!owners.delete(token)) return;
      deactivation ??= cleanup;
      if (owners.size || !deactivation) return;
      const finish = deactivation;
      deactivation = undefined;
      finish();
    },
  };
}
