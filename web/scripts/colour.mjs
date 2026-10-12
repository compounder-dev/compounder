// Contrast arithmetic for the token checks: the standard relative-luminance formula.

/** `#rrggbb` or `rgba(r,g,b,a)` as numbers. Anything else is refused, so a typo cannot pass a check. */
export function parseColour(value) {
  const hex = /^#([0-9a-f]{6})$/i.exec(value);
  if (hex) {
    const n = parseInt(hex[1], 16);
    return { r: n >> 16, g: (n >> 8) & 255, b: n & 255, a: 1 };
  }
  const rgba = /^rgba\((\d+),\s*(\d+),\s*(\d+),\s*([\d.]+)\)$/.exec(value);
  if (rgba) return { r: +rgba[1], g: +rgba[2], b: +rgba[3], a: +rgba[4] };
  throw new Error(`not a colour this check can read: ${value}`);
}

/** The solid colour seen when `fg` is drawn over the solid `bg`. `share` replaces the alpha of `fg`. */
export function over(fg, bg, share = fg.a) {
  const channel = (name) => Math.round(fg[name] * share + bg[name] * (1 - share));
  return { r: channel('r'), g: channel('g'), b: channel('b'), a: 1 };
}

function luminance({ r, g, b }) {
  const linear = (channel) => {
    const c = channel / 255;
    return c <= 0.04045 ? c / 12.92 : ((c + 0.055) / 1.055) ** 2.4;
  };
  return 0.2126 * linear(r) + 0.7152 * linear(g) + 0.0722 * linear(b);
}

/** The contrast ratio of two solid colours, from 1 to 21. */
export function contrast(a, b) {
  const [light, dark] = [luminance(a), luminance(b)].sort((x, y) => y - x);
  return (light + 0.05) / (dark + 0.05);
}

/** The ratio of a token drawn on a ground, with any alpha in the token resolved over that ground. */
export function contrastOn(token, ground) {
  const bg = parseColour(ground);
  return contrast(over(parseColour(token), bg), bg);
}
