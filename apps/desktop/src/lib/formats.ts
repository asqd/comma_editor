// Encodings and delimiters a file can be read with, named as the macOS app names them
// (CSVDocument.encodingName and delimiterName).

import type { TextEncoding } from "./api";
import { L } from "./i18n";

export const encodings: TextEncoding[] = [
  "utf8",
  "utf16Le",
  "utf16Be",
  "windows1251",
  "koi8R",
  "ibm866",
  "koi8U",
  "windows1252",
  "windows1250",
  "windows1253",
  "windows1254",
  "windows1257",
];

export const delimiters = [",", ";", "\t", "|"];

export function encodingName(encoding: TextEncoding): string {
  const names: Record<TextEncoding, string> = {
    utf8: "UTF-8",
    utf16Le: "UTF-16LE",
    utf16Be: "UTF-16BE",
    windows1251: "Windows-1251",
    koi8R: "KOI8-R",
    windows1252: "Windows-1252",
    windows1250: "Windows-1250",
    windows1253: "Windows-1253",
    windows1254: "Windows-1254",
    windows1257: "Windows-1257",
    koi8U: "KOI8-U",
    ibm866: "CP866",
  };
  return names[encoding];
}

export function delimiterName(delimiter: string): string {
  const names: Record<string, string> = {
    ",": L("Comma ( , )"),
    ";": L("Semicolon ( ; )"),
    "\t": L("Tab ( ⇥ )"),
    "|": L("Vertical Bar ( | )"),
  };
  return names[delimiter] ?? delimiter;
}
