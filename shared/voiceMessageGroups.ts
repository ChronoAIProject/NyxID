/** Presentation only. Stored messages and their execution/turn identities stay intact. */
export type VoiceGroupDescription = {
  id: string;
  text: string;
  role: string;
  failed: boolean;
  feedback: boolean;
  voice?: {
    session_id: string;
    segment_id: string;
    sealed: boolean;
    complete: boolean;
    interrupted?: boolean;
  } | null;
  transient?: boolean;
};
export function voiceMessageGroups<T>(
  rows: readonly T[],
  describe: (row: T) => VoiceGroupDescription,
) {
  const groups: { segments: T[]; anchor: T; text: string }[] = [];
  for (const row of rows) {
    const value = describe(row);
    const previous = groups.at(-1);
    const tail = previous ? describe(previous.segments.at(-1)!) : undefined;
    const speech = (value: VoiceGroupDescription | undefined) =>
      value?.role === "assistant" &&
      !!value.voice?.segment_id &&
      !value.failed &&
      !value.transient;
    if (
      previous &&
      speech(value) &&
      speech(tail) &&
      !tail?.voice?.interrupted &&
      tail?.voice?.session_id === value.voice?.session_id
    ) {
      previous.segments.push(row);
      previous.text += `\n\n${value.text}`;
      if (
        value.feedback ||
        !previous.segments
          .slice(0, -1)
          .some((segment) => describe(segment).feedback)
      )
        previous.anchor = row;
    } else groups.push({ segments: [row], anchor: row, text: value.text });
  }
  return groups;
}
