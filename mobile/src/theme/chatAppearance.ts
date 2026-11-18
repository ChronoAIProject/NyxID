type BubbleColors = { readonly background: string; readonly foreground: string };

/** One choice styles user bubbles in both themes; each has a light and a dark version. */
export const bubblePalettes = {
  gray: {
    label: "Gray",
    light: { background: "#E0E0E5", foreground: "#18181B" },
    dark: { background: "#2C2C30", foreground: "#EFEFF1" },
  },
  lavender: {
    label: "Lavender",
    light: { background: "#E4DBFA", foreground: "#281A46" },
    dark: { background: "#3A2E5E", foreground: "#EEE8FF" },
  },
  graphite: {
    label: "Graphite",
    light: { background: "#303036", foreground: "#FFFFFF" },
    dark: { background: "#4A4A52", foreground: "#FFFFFF" },
  },
} as const satisfies Record<string, { label: string; light: BubbleColors; dark: BubbleColors }>;

export type BubblePalette = keyof typeof bubblePalettes;

export function normalizeBubblePalette(value: unknown): BubblePalette {
  return value === "lavender" || value === "graphite" ? value : "gray";
}

export function chatBubbleColors(mode: string, palette: unknown): BubbleColors {
  const choice = bubblePalettes[normalizeBubblePalette(palette)];
  return mode === "dark" ? choice.dark : choice.light;
}

export const userBubbleCorners = {
  borderTopLeftRadius: 18,
  borderTopRightRadius: 18,
  borderBottomLeftRadius: 18,
  borderBottomRightRadius: 4,
};
