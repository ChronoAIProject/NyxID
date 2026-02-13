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
    textClass: "text-[#39594d]",
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
