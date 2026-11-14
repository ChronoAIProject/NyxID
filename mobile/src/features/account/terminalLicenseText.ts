export type TerminalLicenseNotice = {
  packageName: string;
  text: string;
};

/**
 * Keep the generated license bytes untouched while making wrapped upstream
 * paragraphs readable in the narrow native settings view.
 */
export function reflowTerminalLicenses(raw: string): TerminalLicenseNotice[] {
  return raw
    .trim()
    .split(/\n\n(?=@)/)
    .map((section) => {
      const [packageName = "", ...bodyLines] = section.split("\n");
      const text = bodyLines
        .join("\n")
        .split(/\n{2,}/)
        .map((paragraph) => paragraph.split("\n").join(" "))
        .join("\n\n");
      return { packageName, text };
    });
}
