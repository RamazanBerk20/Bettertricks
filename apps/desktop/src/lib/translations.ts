export type SupportedLocale = "en" | "tr" | "es" | "it" | "fr" | "de" | "ru" | "ar" | "zh" | "ja" | "ko";
export type TranslatedLocale = Exclude<SupportedLocale, "en">;
export type LanguagePreference = SupportedLocale | "system";

export interface LocaleMetadata {
  id: SupportedLocale;
  intl: string;
  label: string;
  direction: "ltr" | "rtl";
}

export const SUPPORTED_LOCALES: readonly LocaleMetadata[] = [
  { id: "en", intl: "en", label: "English", direction: "ltr" },
  { id: "tr", intl: "tr", label: "Türkçe", direction: "ltr" },
  { id: "es", intl: "es", label: "Español", direction: "ltr" },
  { id: "it", intl: "it", label: "Italiano", direction: "ltr" },
  { id: "fr", intl: "fr", label: "Français", direction: "ltr" },
  { id: "de", intl: "de", label: "Deutsch", direction: "ltr" },
  { id: "ru", intl: "ru", label: "Русский", direction: "ltr" },
  { id: "ar", intl: "ar", label: "العربية", direction: "rtl" },
  { id: "zh", intl: "zh-CN", label: "简体中文", direction: "ltr" },
  { id: "ja", intl: "ja", label: "日本語", direction: "ltr" },
  { id: "ko", intl: "ko", label: "한국어", direction: "ltr" },
] as const;

export type TranslationDictionary = Record<string, string>;

const TRANSLATION_LOADERS: Record<TranslatedLocale, () => Promise<{ default: TranslationDictionary }>> = {
  tr: () => import("./locales/tr.json"),
  es: () => import("./locales/es.json"),
  it: () => import("./locales/it.json"),
  fr: () => import("./locales/fr.json"),
  de: () => import("./locales/de.json"),
  ru: () => import("./locales/ru.json"),
  ar: () => import("./locales/ar.json"),
  zh: () => import("./locales/zh.json"),
  ja: () => import("./locales/ja.json"),
  ko: () => import("./locales/ko.json"),
};
const translationCache: Partial<Record<TranslatedLocale, Promise<TranslationDictionary>>> = {};

export function loadTranslationDictionary(locale: TranslatedLocale): Promise<TranslationDictionary> {
  const cached = translationCache[locale];
  if (cached) return cached;
  const loading = TRANSLATION_LOADERS[locale]().then((module) => module.default).catch((error) => {
    delete translationCache[locale];
    throw error;
  });
  translationCache[locale] = loading;
  return loading;
}

function matchSupportedLocale(value: string | null | undefined): SupportedLocale | undefined {
  const normalized = value?.trim().toLowerCase().replace("_", "-") ?? "";
  if (normalized.startsWith("zh")) return "zh";
  const language = normalized.split("-")[0];
  return SUPPORTED_LOCALES.some((locale) => locale.id === language)
    ? language as SupportedLocale
    : undefined;
}

export function normalizeLocale(value: string | null | undefined): SupportedLocale {
  return matchSupportedLocale(value) ?? "en";
}

export function normalizeLanguagePreference(value: string | null | undefined): LanguagePreference {
  return value?.trim().toLowerCase() === "system" ? "system" : normalizeLocale(value);
}

export function resolveLanguagePreference(
  value: string | null | undefined,
  systemLanguages: readonly string[] = typeof navigator === "undefined"
    ? []
    : navigator.languages?.length ? navigator.languages : [navigator.language],
): SupportedLocale {
  const preference = normalizeLanguagePreference(value);
  if (preference !== "system") return preference;
  for (const language of systemLanguages) {
    const supported = matchSupportedLocale(language);
    if (supported) return supported;
  }
  return "en";
}

export function localeMetadata(locale: SupportedLocale): LocaleMetadata {
  return SUPPORTED_LOCALES.find((candidate) => candidate.id === locale) ?? SUPPORTED_LOCALES[0];
}
