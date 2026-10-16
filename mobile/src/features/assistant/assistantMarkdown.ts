export const assistantMarkdownOptions = {
  typographer: true,
  linkify: true,
  html: false,
};

export function safeAssistantLinkUrl(url: string): string | null {
  try {
    const parsed = new URL(url);
    if (parsed.protocol === "https:" && !parsed.username && !parsed.password)
      return parsed.href;
  } catch {
    // Unsupported or malformed model-provided destinations remain plain text.
  }
  return null;
}
