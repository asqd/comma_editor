// How a cell's value is laid out: on one line or wrapped onto a few, with invisible characters
// shown or not (InvisibleText.swift and GridViewController's cell drawing in the macOS app).

/** A piece of text and how it is drawn: plain, a mark of an invisible character, or quiet. */
export interface Run {
  text: string;
  style: "text" | "mark" | "quiet";
}

const NO_BREAK_SPACES = new Set([" ", " ", " "]);
const ZERO_WIDTH = new Set(["​", "‌", "‍", "⁠", "﻿"]);

/**
 * The value as runs, with invisible characters made visible: spaces at the edges as ·, tabs as →,
 * no-break spaces as °, zero-width characters as ¦, line breaks as ↵ and an empty value as ∅.
 * Line breaks become `\n` when the cell wraps, so that they break lines there.
 */
export function invisibleRuns(value: string, wraps: boolean): Run[] {
  if (value === "") return [{ text: "∅", style: "quiet" }];
  const runs: Run[] = [];
  const add = (text: string, style: Run["style"]) => {
    const last = runs[runs.length - 1];
    if (last && last.style === style) last.text += text;
    else runs.push({ text, style });
  };
  const characters = [...value];
  // Spaces at the edges: all of them in a value of spaces only.
  let start = 0;
  while (start < characters.length && characters[start] === " ") start += 1;
  let end = characters.length;
  while (end > start && characters[end - 1] === " ") end -= 1;
  for (let index = 0; index < characters.length; index += 1) {
    const character = characters[index];
    if (character === " " && (index < start || index >= end)) add("·", "mark");
    else if (character === "\t") add("→", "mark");
    else if (NO_BREAK_SPACES.has(character)) add("°", "mark");
    else if (ZERO_WIDTH.has(character)) add("¦", "mark");
    else if (character === "\r" || character === "\n") {
      if (character === "\r" && characters[index + 1] === "\n") index += 1;
      add(wraps ? "↵" : " ↵ ", "quiet");
      if (wraps) add("\n", "text");
    } else add(character, "text");
  }
  return runs;
}

/** The value as plain runs: line breaks shown as ↵ on one line, kept when the cell wraps. */
export function plainRuns(value: string, wraps: boolean): Run[] {
  if (wraps) return [{ text: value.replace(/\r\n|\r/g, "\n"), style: "text" }];
  return [{ text: value.replace(/\r\n|\r|\n/g, " ↵ "), style: "text" }];
}

/**
 * Breaks runs into at most `lines` lines no wider than `width`, at spaces where it can and
 * anywhere where a word is too long; the last line ends with … when text is left over.
 */
export function wrap(runs: Run[], width: number, lines: number, measure: (text: string) => number): Run[][] {
  const result: Run[][] = [[]];
  let lineWidth = 0;
  let overflow = false;
  const push = (text: string, style: Run["style"]) => {
    const line = result[result.length - 1];
    const last = line[line.length - 1];
    if (last && last.style === style) last.text += text;
    else line.push({ text, style });
    lineWidth += measure(text);
  };
  const newLine = (): boolean => {
    if (result.length === lines) {
      overflow = true;
      return false;
    }
    result.push([]);
    lineWidth = 0;
    return true;
  };
  outer: for (const run of runs) {
    // Words with the spaces after them, and line breaks on their own.
    for (const piece of run.text.split(/(\n|[^\S\n]+)/)) {
      if (piece === "") continue;
      if (piece === "\n") {
        if (!newLine()) break outer;
        continue;
      }
      const pieceWidth = measure(piece);
      if (lineWidth + pieceWidth <= width || /^\s+$/.test(piece)) {
        push(piece, run.style);
        continue;
      }
      if (lineWidth > 0 && pieceWidth <= width) {
        if (!newLine()) break outer;
        push(piece, run.style);
        continue;
      }
      // A word wider than the cell breaks between characters.
      for (const character of piece) {
        const characterWidth = measure(character);
        if (lineWidth + characterWidth > width && lineWidth > 0 && !newLine()) break outer;
        push(character, run.style);
      }
    }
  }
  if (overflow) {
    const last = result[result.length - 1];
    const tail = last[last.length - 1] ?? { text: "", style: "text" as const };
    if (last.length === 0) last.push(tail);
    tail.text = `${tail.text.replace(/\s+$/, "")}…`;
  }
  return result;
}
