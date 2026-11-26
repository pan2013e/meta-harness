/** Decode a shell's directory notification. This is display data only. */
export function terminalDirectory(value: string, osc: 7 | 9): string | null {
  let path: string;
  if (osc === 9) {
    if (!value.startsWith("9;")) return null;
    path = value.slice(2).replace(/^"(.*)"$/, "$1");
  } else {
    try {
      const uri = new URL(value);
      if (uri.protocol !== "file:") return null;
      path = decodeURIComponent(uri.pathname);
      if (/^\/[a-z]:\//i.test(path)) path = path.slice(1).replaceAll("/", "\\");
    } catch {
      return null;
    }
  }
  if (/[\u0000-\u001f\u007f]/.test(path)) return null;
  return /^(?:\/|[a-z]:[\\/]|\\\\)/i.test(path) ? path : null;
}
