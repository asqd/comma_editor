// The files opened lately, for File ▸ Open Recent, kept for every window of the app.

const KEY = "comma-recent";
const KEPT = 10;

export function recentFiles(): string[] {
  try {
    const stored: unknown = JSON.parse(localStorage.getItem(KEY) ?? "[]");
    return Array.isArray(stored) ? stored.filter((path): path is string => typeof path === "string") : [];
  } catch {
    return [];
  }
}

export function addRecent(path: string): void {
  try {
    localStorage.setItem(KEY, JSON.stringify([path, ...recentFiles().filter((other) => other !== path)].slice(0, KEPT)));
  } catch {
    // Without storage the menu simply stays as it was.
  }
}

export function clearRecent(): void {
  try {
    localStorage.removeItem(KEY);
  } catch {
    // Nothing kept, nothing to clear.
  }
}
