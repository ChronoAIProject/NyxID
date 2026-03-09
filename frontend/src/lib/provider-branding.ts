interface ProviderBrand {
  readonly label: string;
  readonly color: string;
  readonly bgClass: string;
  readonly textClass: string;
  readonly initial: string;
}

const PROVIDER_BRANDS: Readonly<Record<string, ProviderBrand>> = {
  openai: {
    label: "OpenAI",
    color: "#10a37f",
    bgClass: "bg-[#10a37f]/15",
    textClass: "text-[#10a37f]",
    initial: "AI",
  },
  anthropic: {
    label: "Anthropic",
    color: "#d4a27f",
    bgClass: "bg-[#d4a27f]/15",
    textClass: "text-[#d4a27f]",
    initial: "An",
  },
  "google-ai": {
    label: "Google AI",
    color: "#4285f4",
    bgClass: "bg-[#4285f4]/15",
    textClass: "text-[#4285f4]",
    initial: "G",
  },
  mistral: {
    label: "Mistral",
    color: "#f7a832",
    bgClass: "bg-[#f7a832]/15",
    textClass: "text-[#f7a832]",
    initial: "Mi",
  },
  cohere: {
    label: "Cohere",
    color: "#39594d",
    bgClass: "bg-[#39594d]/15",
    // dark:text-white needed because #39594d is too dark for dark backgrounds
    textClass: "text-[#39594d] dark:text-white",
    initial: "Co",
  },
  deepseek: {
    label: "DeepSeek",
    color: "#4D6BFE",
    bgClass: "bg-[#4D6BFE]/15",
    textClass: "text-[#4D6BFE]",
    initial: "DS",
  },
  "openai-codex": {
    label: "Codex",
    color: "#10a37f",
    bgClass: "bg-[#10a37f]/15",
    textClass: "text-[#10a37f]",
    initial: "CX",
  },
  twitter: {
    label: "X (Twitter)",
    color: "#000000",
    bgClass: "bg-[#000000]/15",
    // dark:text-white needed because #000000 is invisible on dark backgrounds
    textClass: "text-[#000000] dark:text-white",
    initial: "X",
  },
  google: {
    label: "Google",
    color: "#4285F4",
    bgClass: "bg-[#4285F4]/15",
    textClass: "text-[#4285F4]",
    initial: "G",
  },
  github: {
    label: "GitHub",
    color: "#181717",
    bgClass: "bg-[#181717]/15",
    textClass: "text-[#181717] dark:text-white",
    initial: "Gh",
  },
  facebook: {
    label: "Facebook",
    color: "#1877F2",
    bgClass: "bg-[#1877F2]/15",
    textClass: "text-[#1877F2]",
    initial: "Fb",
  },
  instagram: {
    label: "Instagram",
    color: "#E4405F",
    bgClass: "bg-[#E4405F]/15",
    textClass: "text-[#E4405F]",
    initial: "Ig",
  },
  linkedin: {
    label: "LinkedIn",
    color: "#0A66C2",
    bgClass: "bg-[#0A66C2]/15",
    textClass: "text-[#0A66C2]",
    initial: "Li",
  },
  discord: {
    label: "Discord",
    color: "#5865F2",
    bgClass: "bg-[#5865F2]/15",
    textClass: "text-[#5865F2]",
    initial: "Dc",
  },
  spotify: {
    label: "Spotify",
    color: "#1DB954",
    bgClass: "bg-[#1DB954]/15",
    textClass: "text-[#1DB954]",
    initial: "Sp",
  },
  slack: {
    label: "Slack",
    color: "#4A154B",
    bgClass: "bg-[#4A154B]/15",
    textClass: "text-[#4A154B] dark:text-white",
    initial: "Sl",
  },
  microsoft: {
    label: "Microsoft",
    color: "#00A4EF",
    bgClass: "bg-[#00A4EF]/15",
    textClass: "text-[#00A4EF]",
    initial: "Ms",
  },
  apple: {
    label: "Apple",
    color: "#000000",
    bgClass: "bg-[#000000]/15",
    textClass: "text-[#000000] dark:text-white",
    initial: "Ap",
  },
  tiktok: {
    label: "TikTok",
    color: "#000000",
    bgClass: "bg-[#000000]/15",
    textClass: "text-[#000000] dark:text-white",
    initial: "Tk",
  },
  twitch: {
    label: "Twitch",
    color: "#9146FF",
    bgClass: "bg-[#9146FF]/15",
    textClass: "text-[#9146FF]",
    initial: "Tw",
  },
  reddit: {
    label: "Reddit",
    color: "#FF4500",
    bgClass: "bg-[#FF4500]/15",
    textClass: "text-[#FF4500]",
    initial: "Re",
  },
};

const DEFAULT_BRAND: ProviderBrand = {
  label: "",
  color: "",
  bgClass: "bg-muted",
  textClass: "text-muted-foreground",
  initial: "?",
};

export function getProviderBrand(slug: string): ProviderBrand {
  return PROVIDER_BRANDS[slug] ?? DEFAULT_BRAND;
}

export function hasKnownBrand(slug: string): boolean {
  return slug in PROVIDER_BRANDS;
}
