import type { VoiceCaption } from "./voice-caption-state";
export type { VoiceCaption, VoiceView } from "./voice-caption-state";

/** Highlight whole provider fragments only; completion is never a word clock. */
export function VoiceCaptionText({
  caption,
  playbackMs,
}: {
  caption: VoiceCaption;
  playbackMs: number;
}) {
  const fragments = caption.fragments;
  const timed =
    !!fragments?.length &&
    fragments.map((fragment) => fragment.text).join("") === caption.text;
  return (
    <div
      className={
        caption.speaker === "user"
          ? "ml-auto max-w-[90%] rounded-xl bg-overlay px-3 py-2 text-13"
          : "ml-[30px] whitespace-pre-wrap px-[7px] text-13 leading-relaxed"
      }
      data-voice-caption={caption.id}
    >
      <span className="sr-only">
        {caption.speaker === "user" ? "You" : "Assistant"}:{" "}
      </span>
      {caption.speaker === "user" ? (
        caption.text
      ) : timed ? (
        fragments.map((fragment, index) => (
          <span
            key={index}
            data-voice-delivery={
              playbackMs >= fragment.end_ms ? "played" : "pending"
            }
            className={
              playbackMs >= fragment.end_ms
                ? "text-foreground"
                : "text-text-tertiary"
            }
          >
            {fragment.text}
          </span>
        ))
      ) : (
        <span
          className={
            caption.end_ms != null &&
            caption.end_ms > 0 &&
            playbackMs >= caption.end_ms
              ? "text-foreground"
              : "text-text-tertiary"
          }
        >
          {caption.text}
        </span>
      )}
      {caption.sealed && !caption.complete && (
        <span className="text-text-tertiary"> (interrupted)</span>
      )}
    </div>
  );
}
