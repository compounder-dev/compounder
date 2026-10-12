// Turns tokens.json into CSS custom properties, written to src/tokens.css.
// Every colour, size and duration a part may use is made here and nowhere else.

import { readFileSync, writeFileSync } from 'node:fs';
import { fileURLToPath } from 'node:url';

export const tokens = JSON.parse(readFileSync(new URL('../tokens.json', import.meta.url), 'utf8'));

const kebab = (name) => name.replace(/[A-Z]/g, (letter) => `-${letter.toLowerCase()}`);
const px = (value) => `${value}px`;
const ms = (value) => `${value}ms`;

/** One theme's colour roles: `text.onAction` becomes `--text-on-action`. */
function colours(theme) {
  const lines = [];
  for (const [group, roles] of Object.entries(tokens.color[theme])) {
    for (const [role, value] of Object.entries(roles)) lines.push([`${group}-${kebab(role)}`, value]);
  }
  lines.push(['elevation-popover', tokens.elevation.popover[theme]]);
  return lines;
}

/** Everything that is the same in both themes. */
function shared() {
  const { type, space, radius, size, motion, z, chart, elevation } = tokens;
  const lines = [['font-sans', type.family.sans]];
  for (const [name, value] of Object.entries(type.weights)) lines.push([`weight-${name}`, value]);
  for (const [name, value] of Object.entries(type.tracking)) lines.push([`tracking-${name}`, value]);
  // One shorthand per step of the scale, so a part cannot set a size without its line height and weight.
  for (const [name, step] of Object.entries(type.scale)) {
    lines.push([`type-${name}`, `${step.weight} ${px(step.size)}/${px(step.line)} var(--font-sans)`]);
    lines.push([`type-${name}-tracking`, `var(--tracking-${step.tracking})`]);
  }
  for (const [step, value] of Object.entries(space.steps)) lines.push([`space-${step}`, px(value)]);
  lines.push(['row-gap', px(space.rowGap)]);
  for (const name of ['control', 'card', 'panel', 'full']) lines.push([`radius-${name}`, px(radius[name])]);
  lines.push(
    ['size-button-md', px(size.button.md)],
    ['size-button-lg', px(size.button.lg)],
    ['size-input', px(size.input)],
    ['size-row', px(size.row)],
    ['size-block', px(size.block)],
    ['size-mark', px(size.mark)],
    ['size-bottom-bar', px(size.bottomBar)],
    ['size-icon-sm', px(size.icon.sm)],
    ['size-icon-md', px(size.icon.md)],
    ['size-icon-lg', px(size.icon.lg)],
    ['size-content-max', px(size.contentMax)],
    ['size-reading-max', px(size.readingMax)],
    ['size-sheet-max', px(size.sheetMax)],
    ['size-dialog-max', px(size.dialogMax)],
    ['size-hairline', px(size.hairline)],
    ['size-focus-width', px(size.focusRing.width)],
    ['size-focus-gap', px(size.focusRing.gap)],
    ['tint-card', `${elevation.cardTintPercent}%`],
  );
  for (const [name, value] of Object.entries(motion.duration)) lines.push([`duration-${kebab(name)}`, ms(value)]);
  for (const [name, value] of Object.entries(motion.easing)) lines.push([`easing-${name}`, value]);
  lines.push(['lift', px(motion.lift)], ['press-scale', motion.pressScale]);
  for (const [name, value] of Object.entries(z)) lines.push([`z-${name}`, value]);
  lines.push(
    ['chart-height-phone', px(chart.height.phone)],
    ['chart-height-desktop', px(chart.height.desktop)],
    ['chart-height-sparkline', px(chart.height.sparkline)],
    ['chart-line-width', px(chart.line.width)],
    ['chart-bar-height', px(chart.allocationBar.height)],
  );
  return lines;
}

/** Values that change with the width of the screen or the kind of pointer. */
function fluid() {
  const { space, size, breakpoint } = tokens;
  return {
    base: [
      ['gutter', px(space.pageGutter.phone)],
      ['card-padding', px(space.cardPadding.phone)],
      ['card-radius', 'var(--radius-card)'],
      ['section-gap', px(space.sectionGap.phone)],
      ['size-top-bar', px(size.topBar.phone)],
      ['size-target', px(size.target.touch)],
    ],
    queries: [
      [`(min-width: ${px(breakpoint.tablet)})`, [['gutter', px(space.pageGutter.tablet)], ['size-top-bar', px(size.topBar.desktop)]]],
      [
        `(min-width: ${px(breakpoint.desktop)})`,
        [
          ['gutter', px(space.pageGutter.desktop)],
          ['card-padding', px(space.cardPadding.desktop)],
          ['card-radius', 'var(--radius-panel)'],
          ['section-gap', px(space.sectionGap.desktop)],
        ],
      ],
      ['(pointer: fine)', [['size-target', px(size.target.finePointer)]]],
    ],
  };
}

const block = (selector, lines, indent = '') =>
  `${indent}${selector} {\n${lines.map(([name, value]) => `${indent}  --${name}: ${value};`).join('\n')}\n${indent}}\n`;

export function css() {
  const { base, queries } = fluid();
  const light = [...colours('light')];
  return [
    '/* Made from tokens.json by scripts/tokens.mjs. Change the JSON, not this file. */\n',
    // Dark is the base. The device's setting picks light, and `data-theme` on the root overrides the device.
    block(':root', [...colours('dark'), ...shared(), ...base]).replace('{\n', '{\n  color-scheme: dark;\n'),
    `@media (prefers-color-scheme: light) {\n${block(":root:not([data-theme='dark'])", light, '  ').replace('{\n', '{\n    color-scheme: light;\n')}}\n`,
    block(":root[data-theme='light']", light).replace('{\n', '{\n  color-scheme: light;\n'),
    ...queries.map(([query, lines]) => `@media ${query} {\n${block(':root', lines, '  ')}}\n`),
  ].join('\n');
}

/** The names of every custom property the token file makes. */
export function tokenNames() {
  return new Set([...css().matchAll(/--([\w-]+):/g)].map((match) => match[1]));
}

if (process.argv[1] === fileURLToPath(import.meta.url)) {
  writeFileSync(new URL('../src/tokens.css', import.meta.url), css());
  console.log(`tokens: wrote src/tokens.css with ${tokenNames().size} custom properties`);
}
