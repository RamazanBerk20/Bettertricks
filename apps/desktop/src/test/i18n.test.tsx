import fs from "node:fs";
import path from "node:path";
import { fireEvent, render, screen } from "@testing-library/react";
import ts from "typescript";

import { I18nProvider, useI18n } from "../lib/i18n";
import {
  loadTranslationDictionary,
  normalizeLanguagePreference,
  normalizeLocale,
  resolveLanguagePreference,
  SUPPORTED_LOCALES,
  type TranslatedLocale,
} from "../lib/translations";

function sourceFiles(directory: string): string[] {
  return fs.readdirSync(directory, { withFileTypes: true }).flatMap((entry) => {
    const filename = path.join(directory, entry.name);
    if (entry.isDirectory()) return entry.name === "test" ? [] : sourceFiles(filename);
    return /\.tsx?$/.test(entry.name) ? [filename] : [];
  });
}

function uiMessageKeys(): string[] {
  const keys = new Set<string>();
  for (const filename of sourceFiles(path.resolve(process.cwd(), "src"))) {
    const source = fs.readFileSync(filename, "utf8");
    const file = ts.createSourceFile(filename, source, ts.ScriptTarget.Latest, true, filename.endsWith("x") ? ts.ScriptKind.TSX : ts.ScriptKind.TS);
    const visit = (node: ts.Node) => {
      if (
        ts.isCallExpression(node)
        && ts.isIdentifier(node.expression)
        && node.expression.text === "t"
        && node.arguments[0]
        && (ts.isStringLiteral(node.arguments[0]) || ts.isNoSubstitutionTemplateLiteral(node.arguments[0]))
      ) keys.add(node.arguments[0].text);
      ts.forEachChild(node, visit);
    };
    visit(file);
  }
  return [...keys].sort();
}

function placeholders(message: string): string[] {
  return [...message.matchAll(/\{[a-zA-Z0-9_]+\}/g)].map((match) => match[0]).sort();
}

function LocaleProbe() {
  const { locale, t, setLocale, formatNumber, formatBytes } = useI18n();
  return (
    <div>
      <span data-testid="locale">{locale}</span>
      <span>{t("Components")}</span>
      <span>{t("{count} selected", { count: formatNumber(1234) })}</span>
      <span>{formatBytes(1536)}</span>
      <button onClick={() => setLocale("ar")}>Arabic</button>
      <button onClick={() => setLocale("zh")}>Chinese</button>
    </div>
  );
}

describe("localization", () => {
  it("ships a complete, placeholder-safe dictionary for every translated locale", async () => {
    const keys = uiMessageKeys();
    expect(keys.length).toBeGreaterThan(350);
    expect(SUPPORTED_LOCALES.map((locale) => locale.id)).toEqual(["en", "tr", "es", "it", "fr", "de", "ru", "ar", "zh", "ja", "ko"]);

    for (const locale of SUPPORTED_LOCALES.filter((entry) => entry.id !== "en")) {
      const dictionary = await loadTranslationDictionary(locale.id as TranslatedLocale);
      expect(Object.keys(dictionary).sort(), `${locale.id} message coverage`).toEqual(keys);
      for (const key of keys) {
        expect(dictionary[key]?.trim(), `${locale.id}: ${key}`).toBeTruthy();
        expect(placeholders(dictionary[key]), `${locale.id}: ${key}`).toEqual(placeholders(key));
      }
    }
  });

  it("normalizes system locale variants and safely falls back to English", () => {
    expect(normalizeLocale("tr_TR.UTF-8")).toBe("tr");
    expect(normalizeLocale("zh-Hans-CN")).toBe("zh");
    expect(normalizeLocale("ar-SA")).toBe("ar");
    expect(normalizeLocale("not-a-locale")).toBe("en");
    expect(normalizeLanguagePreference("SYSTEM")).toBe("system");
    expect(resolveLanguagePreference("system", ["nl-NL", "de-DE"])).toBe("de");
    expect(resolveLanguagePreference("system", ["tr_TR.UTF-8"])).toBe("tr");
    expect(resolveLanguagePreference("system", ["not-a-locale"])).toBe("en");
    expect(resolveLanguagePreference("ja", ["tr-TR"])).toBe("ja");
  });

  it("switches translations, number formatting, and document direction live", async () => {
    const expectedArabicNumber = new Intl.NumberFormat("ar", { maximumFractionDigits: 1 }).format(1234);
    render(<I18nProvider initialLocale="tr"><LocaleProbe /></I18nProvider>);

    expect(await screen.findByText("Bileşenler")).toBeInTheDocument();
    expect(document.documentElement).toHaveAttribute("lang", "tr");
    expect(document.documentElement).toHaveAttribute("dir", "ltr");

    fireEvent.click(screen.getByRole("button", { name: "Arabic" }));
    expect(await screen.findByText("المكونات")).toBeInTheDocument();
    expect(screen.getByText(new RegExp(expectedArabicNumber))).toBeInTheDocument();
    expect(document.documentElement).toHaveAttribute("lang", "ar");
    expect(document.documentElement).toHaveAttribute("dir", "rtl");

    fireEvent.click(screen.getByRole("button", { name: "Chinese" }));
    expect(await screen.findByText("组件")).toBeInTheDocument();
    expect(document.documentElement).toHaveAttribute("lang", "zh-CN");
    expect(document.documentElement).toHaveAttribute("dir", "ltr");
  });
});
