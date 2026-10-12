// The token rule: no raw colour, size or duration in a part. Styles may only
// use the custom properties made from tokens.json. The build fails on a breach.

import { readdirSync, readFileSync } from 'node:fs';
import { join, relative } from 'node:path';
import { fileURLToPath } from 'node:url';
import { tokens, tokenNames } from './tokens.mjs';

const GENERATED = 'tokens.css';

const NAMED_COLOURS =
  'aliceblue antiquewhite aqua aquamarine azure beige bisque black blanchedalmond blue blueviolet brown burlywood cadetblue chartreuse chocolate coral cornflowerblue cornsilk crimson cyan darkblue darkcyan darkgoldenrod darkgray darkgreen darkgrey darkkhaki darkmagenta darkolivegreen darkorange darkorchid darkred darksalmon darkseagreen darkslateblue darkslategray darkslategrey darkturquoise darkviolet deeppink deepskyblue dimgray dimgrey dodgerblue firebrick floralwhite forestgreen fuchsia gainsboro ghostwhite gold goldenrod gray green greenyellow grey honeydew hotpink indianred indigo ivory khaki lavender lavenderblush lawngreen lemonchiffon lightblue lightcoral lightcyan lightgoldenrodyellow lightgray lightgreen lightgrey lightpink lightsalmon lightseagreen lightskyblue lightslategray lightslategrey lightsteelblue lightyellow lime limegreen linen magenta maroon mediumaquamarine mediumblue mediumorchid mediumpurple mediumseagreen mediumslateblue mediumspringgreen mediumturquoise mediumvioletred midnightblue mintcream mistyrose moccasin navajowhite navy oldlace olive olivedrab orange orangered orchid palegoldenrod palegreen paleturquoise palevioletred papayawhip peachpuff peru pink plum powderblue purple rebeccapurple red rosybrown royalblue saddlebrown salmon sandybrown seagreen seashell sienna silver skyblue slateblue slategray slategrey snow springgreen steelblue tan teal thistle tomato turquoise violet wheat white whitesmoke yellow yellowgreen'.split(
    ' ',
  );

const HEX = /#(?:[0-9a-f]{3,4}|[0-9a-f]{6}|[0-9a-f]{8})(?![\w-])/i;
const COLOUR_FUNCTION = /(?<![\w-])(?:rgba?|hsla?|hwb|lab|lch|oklab|oklch|color)\(/i;
const NAMED = new RegExp(`(?<![\\w-])(?:${NAMED_COLOURS.join('|')})(?![\\w-(])`, 'i');
const LENGTH = /(?<![\w.#-])-?(?:\d+\.?\d*|\.\d+)(?:px|rem|em|ex|ch|pt|pc|cm|mm|in|vw|vh|vmin|vmax|dvw|dvh|svw|svh|lvw|lvh)(?![\w-])/gi;
const DURATION = /(?<![\w.#-])(?:\d+\.?\d*|\.\d+)m?s(?![\w-])/i;
// The whole height of the screen is not a design value. Everything else comes from a token.
const ALLOWED_LENGTHS = new Set(['100dvh']);
const TYPE_PROPERTIES = new Set(['font', 'font-family', 'font-size', 'font-weight', 'line-height', 'letter-spacing']);
const TYPE_KEYWORDS = /^(?:inherit|normal|var\(--[\w-]+\))$/;

/** What is wrong with one CSS value, as a list of plain reasons. */
export function valueProblems(property, value) {
  const found = [];
  const bare = value.replace(/var\(--[\w-]+\)/g, 'var()').replace(/(["'])(?:\\.|(?!\1).)*\1/g, '""');
  if (HEX.test(bare) || COLOUR_FUNCTION.test(bare)) found.push('a raw colour');
  else if (!property.startsWith('--') && property !== 'transition' && property !== 'will-change' && NAMED.test(bare)) found.push('a named colour');
  const lengths = (bare.match(LENGTH) ?? []).filter((length) => parseFloat(length) !== 0 && !ALLOWED_LENGTHS.has(length));
  if (lengths.length) found.push(`a raw size (${lengths.join(', ')})`);
  if (DURATION.test(bare)) found.push('a raw duration');
  if (TYPE_PROPERTIES.has(property) && !TYPE_KEYWORDS.test(value.trim())) found.push('type that is not a step of the scale');
  return found;
}

/** The custom properties a file sets for itself, in a stylesheet or in the style of an element. */
export function definedIn(source) {
  return [...source.matchAll(/(?<![\w-])['"]?--([\w-]+)['"]?\s*:/g)].map((match) => match[1]);
}

/** Each breach in one stylesheet: [line, message]. `known` holds the custom properties that exist. */
export function cssProblems(source, known, breakpoints) {
  const found = [];
  const text = source.replace(/\/\*[\s\S]*?\*\//g, (comment) => comment.replace(/[^\n]/g, ' '));
  const lineOf = (index) => text.slice(0, index).split('\n').length;

  for (const match of text.matchAll(/@media([^{]*)\{/g)) {
    for (const width of match[1].matchAll(/(\d+(?:\.\d+)?)px/g)) {
      if (!breakpoints.includes(Number(width[1]))) found.push([lineOf(match.index), `a breakpoint at ${width[0]} that is not in the token file`]);
    }
  }
  // A declaration is `name: value` up to the next `;` or `}`, wherever it sits.
  for (const match of text.matchAll(/(?<=[{;]\s*)(--[\w-]+|[a-z-]+)\s*:\s*([^;{}]+)(?=[;}])/g)) {
    const [, property, value] = match;
    const line = lineOf(match.index);
    for (const reason of valueProblems(property, value)) found.push([line, `${property} uses ${reason}`]);
    for (const used of value.matchAll(/var\(--([\w-]+)/g)) {
      if (!known.has(used[1])) found.push([line, `${property} uses --${used[1]}, which no token makes`]);
    }
  }
  return found;
}

/** Each breach in one script: raw colours, sizes and durations written into the markup. */
export function scriptProblems(source) {
  const found = [];
  source.split('\n').forEach((line, index) => {
    const code = line.replace(/\/\/.*$/, '');
    const reasons = [];
    if (/(["'`])#(?:[0-9a-f]{3,4}|[0-9a-f]{6}|[0-9a-f]{8})\1/i.test(code) || COLOUR_FUNCTION.test(code)) reasons.push('a raw colour');
    if (/(?<![\w.#-])\d+\.?\d*(?:px|rem)(?![\w-])/i.test(code)) reasons.push('a raw size');
    if (/(["'`])\d+\.?\d*m?s\1/i.test(code)) reasons.push('a raw duration');
    for (const reason of reasons) found.push([index + 1, `uses ${reason}`]);
  });
  return found;
}

function files(dir) {
  return readdirSync(dir, { withFileTypes: true }).flatMap((entry) => {
    const path = join(dir, entry.name);
    return entry.isDirectory() ? files(path) : [path];
  });
}

if (process.argv[1] === fileURLToPath(import.meta.url)) {
  const root = fileURLToPath(new URL('../src', import.meta.url));
  const sources = files(root)
    .map((path) => [relative(root, path), path])
    .filter(([name]) => name !== GENERATED && /\.(css|tsx?)$/.test(name))
    .map(([name, path]) => [name, readFileSync(path, 'utf8')]);
  // A part may name a value of its own, such as the fund colour it was handed. Its value still has to be a token.
  const known = new Set([...tokenNames(), ...sources.flatMap(([, source]) => definedIn(source))]);
  const breakpoints = [tokens.breakpoint.tablet, tokens.breakpoint.desktop, tokens.breakpoint.wide];
  const breaches = [];
  let checked = 0;
  for (const [name, source] of sources) {
    const found = name.endsWith('.css') ? cssProblems(source, known, breakpoints) : scriptProblems(source);
    checked += 1;
    for (const [line, message] of found) breaches.push(`src/${name}:${line} ${message}`);
  }
  if (breaches.length) {
    console.error(`styles: ${breaches.length} places use a value that is not a token\n  ${breaches.join('\n  ')}`);
    process.exit(1);
  }
  console.log(`styles: ${checked} files use tokens only`);
}
