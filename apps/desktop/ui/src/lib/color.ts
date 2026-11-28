export function getAccentColorWithAlpha(alpha: number): string {
  const accent = getComputedStyle(document.documentElement)
    .getPropertyValue("--accent")
    .trim();
  const match = /^#([\da-f]{2})([\da-f]{2})([\da-f]{2})$/i.exec(accent);

  if (!match) {
    return `color-mix(in srgb, ${accent} ${alpha * 100}%, transparent)`;
  }

  const [, red, green, blue] = match;
  return `rgba(${parseInt(red, 16)}, ${parseInt(green, 16)}, ${parseInt(blue, 16)}, ${alpha})`;
}
