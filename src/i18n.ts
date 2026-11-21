import { useSyncExternalStore } from "react";

// English source strings are the keys: a missing translation falls back to
// readable English, and `scripts/i18n-check.mjs` finds every key statically.
export const locales = {
  en: "English",
  pl: "Polski",
  de: "Deutsch",
  ru: "Русский",
  uk: "Українська",
  it: "Italiano",
  es: "Español",
} as const;
export type Locale = keyof typeof locales;

type Plural = Partial<Record<Intl.LDMLPluralRule, string>> & { other: string };
type Dictionary = Record<string, string | Plural>;
type Vars = Record<string, string | number>;

const STORAGE_KEY = "lomi.locale.v1";
// Vite rewrites the glob call and defines import.meta.env; plain Node (unit
// tests) has neither, so it runs without dictionaries.
const loaders: Record<string, () => Promise<Dictionary>> = import.meta.env
  ? import.meta.glob<Dictionary>("./locales/*.json", { import: "default" })
  : {};

let locale: Locale = "en";
let dictionary: Dictionary = {};
let pluralRules = new Intl.PluralRules("en");
const listeners = new Set<() => void>();

const isLocale = (value: unknown): value is Locale =>
  typeof value === "string" && Object.hasOwn(locales, value);

/** First supported system language, otherwise English. */
export function detectLocale(languages: readonly string[]): Locale {
  for (const language of languages) {
    const base = language.toLowerCase().split(/[-_]/)[0];
    if (isLocale(base)) return base;
  }
  return "en";
}

function stored(): Locale | null {
  try {
    const value = localStorage.getItem(STORAGE_KEY);
    return isLocale(value) ? value : null;
  } catch {
    return null;
  }
}

async function load(next: Locale) {
  const loader = loaders[`./locales/${next}.json`];
  activateLocale(next, next === "en" || !loader ? {} : await loader());
  document.documentElement.lang = next;
}

/** Activates a dictionary; exported for tests. */
export function activateLocale(next: Locale, entries: Dictionary) {
  dictionary = entries;
  locale = next;
  pluralRules = new Intl.PluralRules(next);
  for (const listener of listeners) listener();
}

/**
 * Resolves the language before the first render. The system language is
 * consulted only on the first launch; the result is stored so later system
 * changes never switch the interface unexpectedly.
 */
export async function initializeLocale() {
  let initial = stored();
  if (!initial) {
    initial = detectLocale(navigator.languages ?? [navigator.language]);
    saveLocale(initial);
  }
  await load(initial).catch(() => load("en"));
}

function saveLocale(next: Locale) {
  try {
    localStorage.setItem(STORAGE_KEY, next);
  } catch {
    // Without storage the language is detected again on the next launch.
  }
}

export async function setLocale(next: Locale) {
  saveLocale(next);
  await load(next);
}

export const currentLocale = () => locale;

const fill = (text: string, vars?: Vars) =>
  vars
    ? text.replace(/\{(\w+)\}/g, (match, name: string) =>
        name in vars ? String(vars[name]) : match,
      )
    : text;

/** Translates an English source string; `{name}` placeholders are filled. */
export function t(key: string, vars?: Vars): string {
  const entry = dictionary[key];
  return fill(typeof entry === "string" ? entry : (entry?.other ?? key), vars);
}

/**
 * Count-dependent text. `other` is the key; translations may provide every
 * CLDR category their language needs (one/few/many/other).
 */
export function tp(count: number, one: string, other: string, vars?: Vars) {
  const entry = dictionary[other];
  const all = { count, ...vars };
  if (entry === undefined || typeof entry === "string")
    return fill(count === 1 ? one : (entry ?? other), all);
  return fill(entry[pluralRules.select(count)] ?? entry.other, all);
}

/** Marks a constant for extraction; translate it with `t()` when rendering. */
export const msg = <T extends string>(text: T) => text;

const subscribe = (listener: () => void) => {
  listeners.add(listener);
  return () => listeners.delete(listener);
};

/** Re-renders the component when the language changes. */
export function useT() {
  const current = useSyncExternalStore(subscribe, currentLocale);
  return { t, tp, locale: current };
}
