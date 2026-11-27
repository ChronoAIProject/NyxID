import chinese from "./home-copy.json";

export function homeText(english: keyof typeof chinese, language: "en" | "zh") {
  return language === "zh" ? chinese[english] : english;
}
