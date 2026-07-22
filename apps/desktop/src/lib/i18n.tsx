import { createContext, useCallback, useContext, useLayoutEffect, useMemo, useRef, useState } from "react";
import type { ReactNode } from "react";
import {
  localeMetadata,
  loadTranslationDictionary,
  normalizeLocale,
  type SupportedLocale,
  type TranslationDictionary,
} from "./translations";

export type TranslationValues = Record<string, string | number>;

interface I18nContextValue {
  locale: SupportedLocale;
  intlLocale: string;
  direction: "ltr" | "rtl";
  setLocale: (locale: SupportedLocale) => void;
  t: (key: string, values?: TranslationValues) => string;
  formatNumber: (value: number) => string;
  formatBytes: (value: number | null | undefined) => string;
  formatRelativeTime: (value: string | null) => string;
  formatTime: (value: string) => string;
}

function interpolate(message: string, values?: TranslationValues, isolateValues = false): string {
  if (!values) return message;
  return message.replace(/\{([a-zA-Z0-9_]+)\}/g, (placeholder, key: string) => (
    values[key] === undefined
      ? placeholder
      : isolateValues ? `\u2068${String(values[key])}\u2069` : String(values[key])
  ));
}

function createTranslator(locale: SupportedLocale, dictionary?: TranslationDictionary) {
  return (key: string, values?: TranslationValues) => {
    const translated = locale === "en" ? key : dictionary?.[key] ?? key;
    return interpolate(translated, values, locale === "ar");
  };
}

function createFormatters(intlLocale: string, t: I18nContextValue["t"]): Pick<I18nContextValue, "formatNumber" | "formatBytes" | "formatRelativeTime" | "formatTime"> {
  const numberFormatter = new Intl.NumberFormat(intlLocale, { maximumFractionDigits: 1 });
  const relativeFormatter = new Intl.RelativeTimeFormat(intlLocale, { numeric: "auto", style: "short" });
  const timeFormatter = new Intl.DateTimeFormat(intlLocale, { hour: "2-digit", minute: "2-digit", second: "2-digit" });
  const dateFormatter = new Intl.DateTimeFormat(intlLocale, { dateStyle: "medium" });
  return {
    formatNumber: (number) => numberFormatter.format(number),
    formatBytes: (bytes) => {
      if (bytes == null) return t("Unknown");
      const units = ["B", "KB", "MB", "GB", "TB"];
      let amount = bytes;
      let unit = 0;
      while (amount >= 1024 && unit < units.length - 1) {
        amount /= 1024;
        unit += 1;
      }
      return `${numberFormatter.format(unit === 0 ? Math.round(amount) : amount)} ${units[unit]}`;
    },
    formatRelativeTime: (date) => {
      if (!date) return t("Never");
      const difference = Date.now() - new Date(date).getTime();
      const minutes = Math.round(difference / 60_000);
      if (Math.abs(minutes) < 60) return relativeFormatter.format(-minutes, "minute");
      const hours = Math.round(minutes / 60);
      if (Math.abs(hours) < 24) return relativeFormatter.format(-hours, "hour");
      const days = Math.round(hours / 24);
      if (Math.abs(days) < 30) return relativeFormatter.format(-days, "day");
      return dateFormatter.format(new Date(date));
    },
    formatTime: (date) => timeFormatter.format(new Date(date)),
  };
}

const englishMetadata = localeMetadata("en");
const englishTranslator = createTranslator("en");
const defaultContext: I18nContextValue = {
  locale: "en",
  intlLocale: englishMetadata.intl,
  direction: "ltr",
  setLocale: () => undefined,
  t: englishTranslator,
  ...createFormatters(englishMetadata.intl, englishTranslator),
};

const I18nContext = createContext<I18nContextValue>(defaultContext);

export function I18nProvider({ children, initialLocale = "en" }: { children: ReactNode; initialLocale?: string }) {
  const [localeState, setLocaleState] = useState<{ locale: SupportedLocale; dictionary?: TranslationDictionary }>({ locale: "en" });
  const requestId = useRef(0);
  const { locale, dictionary } = localeState;
  const metadata = localeMetadata(locale);
  const t = useMemo(() => createTranslator(locale, dictionary), [dictionary, locale]);
  const setLocale = useCallback((requestedLocale: SupportedLocale) => {
    const nextLocale = normalizeLocale(requestedLocale);
    const currentRequest = ++requestId.current;
    if (nextLocale === "en") {
      setLocaleState({ locale: "en" });
      return;
    }
    void loadTranslationDictionary(nextLocale).then((nextDictionary) => {
      if (requestId.current === currentRequest) setLocaleState({ locale: nextLocale, dictionary: nextDictionary });
    }).catch(() => {
      if (requestId.current === currentRequest) setLocaleState({ locale: "en" });
    });
  }, []);

  useLayoutEffect(() => {
    const normalizedInitialLocale = normalizeLocale(initialLocale);
    if (normalizedInitialLocale !== "en") setLocale(normalizedInitialLocale);
  }, [initialLocale, setLocale]);

  useLayoutEffect(() => {
    const previousLanguage = document.documentElement.getAttribute("lang");
    const previousDirection = document.documentElement.getAttribute("dir");
    const previousLocale = document.documentElement.dataset.locale;
    document.documentElement.lang = metadata.intl;
    document.documentElement.dir = metadata.direction;
    document.documentElement.dataset.locale = locale;
    return () => {
      if (previousLanguage === null) document.documentElement.removeAttribute("lang");
      else document.documentElement.lang = previousLanguage;
      if (previousDirection === null) document.documentElement.removeAttribute("dir");
      else document.documentElement.dir = previousDirection;
      if (previousLocale === undefined) delete document.documentElement.dataset.locale;
      else document.documentElement.dataset.locale = previousLocale;
    };
  }, [locale, metadata.direction, metadata.intl]);

  const value = useMemo<I18nContextValue>(() => {
    return {
      locale,
      intlLocale: metadata.intl,
      direction: metadata.direction,
      setLocale,
      t,
      ...createFormatters(metadata.intl, t),
    };
  }, [locale, metadata.direction, metadata.intl, setLocale, t]);

  return <I18nContext.Provider value={value}>{children}</I18nContext.Provider>;
}

export function useI18n(): I18nContextValue {
  return useContext(I18nContext);
}
