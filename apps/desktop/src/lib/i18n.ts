// The interface's words, in English or Russian as the system's language is. As in the macOS
// app, the English text is the key; Russian comes from the macOS app's own Localizable.strings,
// so both apps say the same thing, and from `desktop` for what only this app says.

import macStrings from "../../../macos/CommaEditor/Resources/ru.lproj/Localizable.strings?raw";

/** Russian for words only this app has. */
const desktop: Record<string, string> = {
  "Open a CSV file: %@": "Откройте CSV-файл: %@",
  "Scroll Bench": "Замер прокрутки",
  "Timing…": "Замер…",
  "Copy": "Скопировать",
  "Close": "Закрыть",
  "Open": "Открыть",
  "Demo in a browser: rows are made up as they are shown": "Демо в браузере: строки придумываются на лету",
  "opened in %@ s": "открытие %@ с",
  "first screen in %@ s": "первый экран %@ с",
  "frame p95 %@ ms": "кадр p95 %@ мс",
  "request p95 %@ ms": "запрос p95 %@ мс",
  "Edited": "Изменён",
  "Restore the unsaved changes to “%@”?": "Восстановить несохранённые изменения в «%@»?",
  "CommaEditor closed before they were saved. Restored changes open in a window; save them to keep them.":
    "CommaEditor закрылся, не сохранив их. Восстановленные изменения откроются в окне; сохраните их, чтобы не потерять.",
  "Restore": "Восстановить",
  "Nothing to fix": "Исправлять нечего",
  "Discard": "Удалить",
  "CSV and Excel": "CSV и Excel",
  "Download page": "Страница загрузки",
  "An editor of CSV files and Excel workbooks that keeps files as they were": "Редактор CSV и книг Excel, который не портит файлы",
  // What AppKit says for the macOS app.
  "Do you want to save the changes made to the document “%@”?": "Сохранить изменения в документе «%@»?",
  "Your changes will be lost if you don’t save them.": "Если не сохранить их, изменения будут потеряны.",
  "Don’t Save": "Не сохранять",
};

function parseStrings(text: string): Record<string, string> {
  const unescape = (value: string) =>
    value.replace(/\\(.)/g, (_, character: string) => ({ n: "\n", t: "\t", r: "\r" })[character] ?? character);
  const entries: Record<string, string> = {};
  for (const match of text.matchAll(/"((?:[^"\\]|\\.)*)"\s*=\s*"((?:[^"\\]|\\.)*)"\s*;/g)) {
    entries[unescape(match[1])] = unescape(match[2]);
  }
  return entries;
}

const language = new URLSearchParams(location.search).get("lang") ?? navigator.language;
export const russian = language.toLowerCase().startsWith("ru");
const dictionary: Record<string, string> = russian ? { ...parseStrings(macStrings), ...desktop } : {};

/** The words for `key` with `%@`, `%ld`, `%lld` and `%d` filled in turn from `values`. */
export function L(key: string, ...values: (string | number)[]): string {
  let next = 0;
  return (dictionary[key] ?? key).replace(/%(?:(\d+)\$)?(?:@|l{0,2}d|lu)|%%/g, (match, position?: string) => {
    if (match === "%%") return "%";
    const value = position ? values[Number(position) - 1] : values[next++];
    return String(value ?? "");
  });
}

export const locale = russian ? "ru-RU" : "en-US";
const integers = new Intl.NumberFormat(locale);
export const count = (value: number) => integers.format(value);
