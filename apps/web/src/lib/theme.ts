/** Accent for spaces that never picked one (matches the server default). */
export const DEFAULT_ACCENT = '#ff6b35';
/** Accent choices offered in Settings → Appearance. */
export const ACCENTS = ['#ff6b35', '#e8b02e', '#3fb27f', '#38a8d6', '#8b7cf6', '#e0569b'];

/** Text color for a filled background: near-black on light colors, white on deep ones. */
export function inkFor(hex: string): string {
  const value = hex.replace('#', '');
  const full = value.length === 3 ? [...value].map((c) => c + c).join('') : value.padEnd(6, '0');
  const n = parseInt(full.slice(0, 6), 16);
  if (Number.isNaN(n)) return '#ffffff';
  const [r, g, b] = [n >> 16, (n >> 8) & 255, n & 255].map((c) => {
    const v = c / 255;
    return v <= 0.03928 ? v / 12.92 : ((v + 0.055) / 1.055) ** 2.4;
  });
  return 0.2126 * r + 0.7152 * g + 0.0722 * b > 0.2 ? '#16130f' : '#ffffff';
}
