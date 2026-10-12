// The weight budget, measured on what the build made. A public screen may load at
// most the script and CSS that tokens.json allows, compressed, one font file, and
// nothing from another origin. Over budget, the build fails.
//
// Every screen is drawn by the one entry the page names, so what the page loads is
// what every screen loads. The engine is the exception: it is fetched by the one
// screen that runs it, and it is measured here on its own line.
//
// This also leaves a compressed copy beside each asset, which the server sends to a
// browser that asks for one. The sizes checked are the sizes sent.

import { readdirSync, readFileSync, writeFileSync } from 'node:fs';
import { fileURLToPath } from 'node:url';
import { brotliCompressSync, constants, gzipSync } from 'node:zlib';
import { tokens } from './tokens.mjs';

const KB = 1024;

/** What a page's own markup makes a browser load, by kind. */
export function loadedBy(html) {
  const tags = [...html.matchAll(/<(script|link)\b[^>]*>/g)].map(([tag, name]) => {
    const attribute = (key) => new RegExp(`\\b${key}="([^"]*)"`).exec(tag)?.[1];
    return { name, rel: attribute('rel'), as: attribute('as'), type: attribute('type'), url: attribute('src') ?? attribute('href') };
  });
  const scripts = tags.filter((tag) => (tag.name === 'script' && tag.type !== 'application/json') || tag.rel === 'modulepreload');
  return {
    scripts: scripts.map((tag) => tag.url),
    styles: tags.filter((tag) => tag.rel === 'stylesheet').map((tag) => tag.url),
    fonts: tags.filter((tag) => tag.rel === 'preload' && tag.as === 'font').map((tag) => tag.url),
  };
}

/** The font faces a stylesheet declares: the file of each and the characters it is for, when it says. */
export function fontFaces(sheet) {
  return [...sheet.matchAll(/@font-face\s*\{([^}]*)\}/g)].map(([, face]) => ({
    url: /url\(["']?([^"')]+)/.exec(face)?.[1],
    ranges: /unicode-range:\s*([^;}]+)/i.exec(face)?.[1].split(',').map((range) => {
      const [from, to = from] = range.trim().replace(/^U\+/i, '').split('-');
      return [parseInt(from, 16), parseInt(to, 16)];
    }),
  }));
}

/**
 * Why a page would fetch more than its one font file, if it would. Every other face must
 * be fenced to characters of its own, and no character in `text` may fall outside the first.
 */
export function extraFonts(faces, early, text) {
  const first = faces.find((face) => face.url === early);
  if (!first) return [`no font face uses ${early}, the file fetched early`];
  const found = faces.filter((face) => face !== first && !face.ranges).map((face) => `${face.url} has no unicode-range, so every page would fetch it`);
  const covered = (code) => !first.ranges || first.ranges.some(([from, to]) => code >= from && code <= to);
  const strays = [...new Set(text)].filter((character) => !covered(character.codePointAt(0)));
  if (strays.length) found.push(`the words of the app use ${strays.map((c) => `${c} (U+${c.codePointAt(0).toString(16).toUpperCase()})`).join(', ')}, which the one font file does not hold`);
  return found;
}

/** A script or sheet from our own build, as against one named by a full address. */
export const isOurs = (url) => typeof url === 'string' && /^\/assets\/[\w.-]+$/.test(url);

/** Each failure of the budget, in plain words. `sizes` are compressed bytes. */
export function overBudget(budget, sizes) {
  const found = [];
  const kb = (bytes) => `${(bytes / KB).toFixed(1)} KB`;
  if (sizes.script > budget.publicPageScriptGzipKB * KB) found.push(`script is ${kb(sizes.script)}, over ${budget.publicPageScriptGzipKB} KB`);
  if (sizes.css > budget.publicPageCssGzipKB * KB) found.push(`CSS is ${kb(sizes.css)}, over ${budget.publicPageCssGzipKB} KB`);
  if (sizes.fonts !== budget.fontFiles) found.push(`${sizes.fonts} font files are fetched early, not ${budget.fontFiles}`);
  if (sizes.outside > budget.thirdPartyScriptsOnPublicPages) found.push(`${sizes.outside} scripts or sheets come from outside the build`);
  return found;
}

if (process.argv[1] === fileURLToPath(import.meta.url)) {
  const dist = new URL('../dist/', import.meta.url);
  const read = (url) => readFileSync(new URL(`.${url}`, dist));
  const gzip = (bytes) => gzipSync(bytes, { level: 9 });
  const brotli = (bytes) => brotliCompressSync(bytes, { params: { [constants.BROTLI_PARAM_QUALITY]: 11, [constants.BROTLI_PARAM_SIZE_HINT]: bytes.length } });
  const kb = (bytes) => `${(bytes / KB).toFixed(1)} KB`.padStart(9);

  const loaded = loadedBy(readFileSync(new URL('index.html', dist), 'utf8'));
  const outside = [...loaded.scripts, ...loaded.styles, ...loaded.fonts].filter((url) => !isOurs(url));
  const ours = (urls) => urls.filter(isOurs);
  const total = (urls) => ours(urls).reduce((sum, url) => sum + gzip(read(url)).length, 0);

  const assets = readdirSync(new URL('assets/', dist)).filter((name) => !/\.(br|gz)$/.test(name));
  const first = new Set([...loaded.scripts, ...loaded.styles].map((url) => url.replace('/assets/', '')));
  // Whatever script the page does not name is fetched later, by the screen that needs it.
  const later = assets.filter((name) => /\.(js|wasm)$/.test(name) && !first.has(name));
  const firstScript = ours(loaded.scripts).map((url) => read(url).toString('utf8')).join('\n');
  const failures = overBudget(tokens.budget, { script: total(loaded.scripts), css: total(loaded.styles), fonts: loaded.fonts.length, outside: outside.length });
  for (const name of later.filter((each) => each.endsWith('.wasm'))) {
    if (firstScript.includes(name)) failures.push(`the first script names ${name}, so every screen would fetch the engine`);
  }
  if (!later.some((name) => name.endsWith('.wasm'))) failures.push('the engine is missing from the build');
  // One font file means one: the others wait for characters that the app's own words never use.
  const sheets = ours(loaded.styles).map((url) => read(url).toString('utf8')).join('\n');
  if (loaded.fonts.length === 1) failures.push(...extraFonts(fontFaces(sheets), loaded.fonts[0], firstScript));

  // Fonts are compressed already. Everything else gets a copy the server can send as it is.
  for (const name of assets.filter((each) => /\.(js|css|wasm|svg|json)$/.test(each))) {
    const bytes = read(`/assets/${name}`);
    writeFileSync(new URL(`assets/${name}.gz`, dist), gzip(bytes));
    writeFileSync(new URL(`assets/${name}.br`, dist), brotli(bytes));
  }

  const { budget } = tokens;
  console.log('budget: what every screen loads, compressed');
  console.log(`  script ${kb(total(loaded.scripts))}  of ${budget.publicPageScriptGzipKB} KB   ${loaded.scripts.join(' ')}`);
  console.log(`  CSS    ${kb(total(loaded.styles))}  of ${budget.publicPageCssGzipKB} KB   ${loaded.styles.join(' ')}`);
  console.log(`  fonts  ${String(loaded.fonts.length).padStart(6)}     of ${budget.fontFiles}       ${loaded.fonts.map((url) => `${url} ${kb(read(url).length).trim()}`).join(' ')}`);
  console.log(`  from outside the build: ${outside.length}`);
  console.log('budget: fetched only by the screen that runs the engine');
  for (const name of later) console.log(`  ${kb(gzip(read(`/assets/${name}`)).length)}  ${name}  (${kb(brotli(read(`/assets/${name}`)).length).trim()} in the smaller of the two copies the server can send)`);
  if (failures.length) {
    console.error(`budget: over\n  ${failures.join('\n  ')}`);
    process.exit(1);
  }
  console.log('budget: held');
}
