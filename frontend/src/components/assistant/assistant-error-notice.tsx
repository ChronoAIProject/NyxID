/** Kept outside the transcript scroller so failures stay visible above the chat. */
export function AssistantErrorNotice({ message }: { readonly message?: string }) {
  if (!message) return null;
  return (
    <div className="shrink-0 px-4 pt-3 sm:px-6">
      <p role="alert" className="mx-auto w-full max-w-[758px] whitespace-pre-line break-words rounded-xl border border-destructive/25 bg-destructive/5 px-4 py-3 text-12 text-destructive">
        {message}
      </p>
    </div>
  );
}
