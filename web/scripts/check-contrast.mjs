// Works out every text and edge pair in tokens.json again and fails the build
// if one is under its mark, or if the table no longer matches the colours.

import { fileURLToPath } from 'node:url';
import { contrast, contrastOn, over, parseColour } from './colour.mjs';
import { tokens } from './tokens.mjs';

/** The ground a row names by its short key: a page ground, a button fill or a rule fill. */
function ground(colour, key) {
  return colour.ground[key] ?? colour.action[key] ?? colour.block[key];
}

/** The value of the role a row starts with, such as `text.primary` or `fund.blue`. */
function role(colour, pair) {
  const [group, name] = pair.split(' ')[0].split('.');
  return colour[group]?.[name];
}

export function problems(file = tokens) {
  const found = [];
  for (const theme of file.meta.themes) {
    const colour = file.color[theme];
    for (const row of file.contrast[theme]) {
      const label = `${theme}: ${row.pair}`;
      if (role(colour, row.pair) !== row.token) {
        found.push(`${label}: the table tests ${row.token} but the colour is now ${role(colour, row.pair)}`);
        continue;
      }
      for (const [key, stated] of Object.entries(row.ratios)) {
        const on = ground(colour, key);
        if (!on) {
          found.push(`${label}: no ground called ${key}`);
          continue;
        }
        const ratio = contrastOn(row.token, on);
        if (ratio < row.needs) found.push(`${label} on ${key}: ${ratio.toFixed(2)} is under ${row.needs}`);
        else if (Math.abs(ratio - stated) > 0.02) found.push(`${label} on ${key}: the table says ${stated}, the colours give ${ratio.toFixed(2)}`);
      }
    }
    // A strategy card is washed with its fund colour. Text must still pass on the strongest wash.
    const raised = parseColour(colour.ground.raised);
    for (const [name, value] of Object.entries(colour.fund)) {
      const washed = over(parseColour(value), raised, file.elevation.cardTintPercent / 100);
      for (const level of ['primary', 'muted', 'faint']) {
        const ratio = contrast(over(parseColour(colour.text[level]), washed), washed);
        if (ratio < 4.5) found.push(`${theme}: text.${level} on a card washed with fund.${name}: ${ratio.toFixed(2)} is under 4.5`);
      }
    }
  }
  return found;
}

if (process.argv[1] === fileURLToPath(import.meta.url)) {
  const found = problems();
  const pairs = tokens.meta.themes.reduce((sum, theme) => sum + tokens.contrast[theme].length, 0);
  if (found.length) {
    console.error(`contrast: ${found.length} of the pairs in tokens.json fail\n  ${found.join('\n  ')}`);
    process.exit(1);
  }
  console.log(`contrast: ${pairs} pairs pass in ${tokens.meta.themes.join(' and ')}, and text passes on every washed card`);
}
