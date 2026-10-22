// Every file in i18n/zh exports a default English -> Chinese map; one file per feature.
const zh: Record<string, string> = Object.assign(
  {},
  ...Object.values(import.meta.glob<Record<string, string>>("./i18n/zh/*.ts", { eager: true, import: "default" })),
);

/** English source text is the key; Chinese comes from `zh` when the UI language is Chinese. */
let chinese = navigator.language.toLowerCase().startsWith("zh");

export function setLanguage(language: "en" | "zh-CN" | "system") {
  chinese = language === "zh-CN" || (language === "system" && navigator.language.toLowerCase().startsWith("zh"));
}

export function t(text: string, values?: Record<string, string | number>): string {
  let result = (chinese && zh[text]) || text;
  for (const [key, value] of Object.entries(values ?? {})) result = result.replaceAll(`{${key}}`, String(value));
  return result;
}
