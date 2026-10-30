export const lightBubblePalettes = {
  gray: { label: "Gray", background: "#E0E0E5", foreground: "#18181B" },
  lavender: { label: "Lavender", background: "#E4DBFA", foreground: "#281A46" },
  graphite: { label: "Graphite", background: "#303036", foreground: "#FFFFFF" },
} as const;

export type LightBubblePalette = keyof typeof lightBubblePalettes;

export function normalizeLightBubblePalette(value: unknown): LightBubblePalette {
  return value === "lavender" || value === "graphite" ? value : "gray";
}

export function chatBubbleColors(mode: string, palette: unknown) {
  return mode === "dark"
    ? { background: "#2C2C30", foreground: "#EFEFF1" }
    : lightBubblePalettes[normalizeLightBubblePalette(palette)];
}

export const userBubbleCorners = {
  borderTopLeftRadius: 18,
  borderTopRightRadius: 18,
  borderBottomLeftRadius: 18,
  borderBottomRightRadius: 4,
};
