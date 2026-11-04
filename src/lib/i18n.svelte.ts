import { invoke } from "@tauri-apps/api/core";
import { en } from "./locales/en";

export type Locale = "zh-CN" | "en-US";

const STORAGE_KEY = "lsm.language";

function readCached(): Locale {
  try {
    return localStorage.getItem(STORAGE_KEY) === "en-US" ? "en-US" : "zh-CN";
  } catch {
    return "zh-CN";
  }
}

/** Reactive locale holder. Read `i18n.locale` (directly or through `t`) inside templates to re-render on change. */
export const i18n = $state<{ locale: Locale }>({ locale: readCached() });

function apply(locale: Locale) {
  i18n.locale = locale;
  document.documentElement.lang = locale;
  try {
    localStorage.setItem(STORAGE_KEY, locale);
  } catch {
    /* storage unavailable: the backend setting remains the source of truth */
  }
}

apply(i18n.locale);

export function setLocale(value: string | null | undefined) {
  const next: Locale = value === "en-US" ? "en-US" : "zh-CN";
  if (next !== i18n.locale) apply(next);
}

/** Loads the stored language preference from the backend. */
export async function initLocale() {
  try {
    const settings = await invoke<Record<string, string>>("get_settings");
    setLocale(settings.language);
  } catch {
    /* keep cached locale */
  }
}

function interpolate(text: string, params?: Record<string, string | number>): string {
  if (!params) return text;
  return text.replace(/\{(\w+)\}/g, (match, name: string) =>
    name in params ? String(params[name]) : match,
  );
}

/**
 * Translates a Chinese source string. Chinese is the key and the zh-CN text;
 * en-US looks it up in the English dictionary and falls back to the key.
 */
export function t(key: string, params?: Record<string, string | number>): string {
  const text = i18n.locale === "en-US" ? (en[key] ?? key) : key;
  return interpolate(text, params);
}

const BACKEND_PATTERNS = Object.entries(en).filter(([key]) => key.includes("{"));

/**
 * Translates an error/message string produced by the Rust backend. Tries an
 * exact match, then `{placeholder}` templates, then the text before the first
 * colon with the remainder kept verbatim (e.g. "读取日志失败: os error 2").
 */
export function tBackend(message: string): string {
  if (i18n.locale !== "en-US") return message;
  const exact = en[message];
  if (exact !== undefined) return exact;

  for (const [key, value] of BACKEND_PATTERNS) {
    const source = key
      .split(/\{\w+\}/)
      .map((part) => part.replace(/[.*+?^${}()|[\]\\]/g, "\\$&"))
      .join("(.+?)");
    const names = [...key.matchAll(/\{(\w+)\}/g)].map((m) => m[1]);
    const match = new RegExp(`^${source}$`, "s").exec(message);
    if (match) {
      const params: Record<string, string> = {};
      names.forEach((name, index) => (params[name] = match[index + 1]));
      return interpolate(value, params);
    }
  }

  const split = message.search(/[:：]/);
  if (split > 0) {
    const head = message.slice(0, split);
    const translated = en[head];
    if (translated !== undefined) return `${translated}${message[split] === "：" ? "：" : ":"}${message.slice(split + 1)}`;
  }
  return message;
}
