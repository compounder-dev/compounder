// The build's own checks, checked: each one must refuse what it exists to refuse.

import assert from 'node:assert/strict';
import { test } from 'node:test';
import { extraFonts, fontFaces, isOurs, loadedBy, overBudget } from './check-budget.mjs';
import { problems } from './check-contrast.mjs';
import { cssProblems, definedIn, scriptProblems, valueProblems } from './check-styles.mjs';
import { contrast, contrastOn, parseColour } from './colour.mjs';
import { css, tokenNames, tokens } from './tokens.mjs';

const known = tokenNames();
const breakpoints = [640, 1024, 1280];
const messages = (source) => cssProblems(source, known, breakpoints).map(([, message]) => message);

test('the token stylesheet has every role in both themes', () => {
  const sheet = css();
  for (const name of ['ground-page', 'text-on-action', 'action-fill-hover', 'status-loss-wash', 'fund-magenta', 'type-caption', 'space-16', 'radius-panel', 'duration-sheet-in', 'easing-exit', 'z-toast', 'size-focus-gap']) {
    assert.ok(known.has(name), name);
  }
  const light = sheet.slice(sheet.indexOf("[data-theme='light']"));
  assert.match(sheet, /:root \{\n {2}color-scheme: dark;\n {2}--ground-page: #1f1f1f;/);
  assert.match(sheet, /@media \(prefers-color-scheme: light\) \{\n {2}:root:not\(\[data-theme='dark'\]\) \{\n {4}color-scheme: light;\n {4}--ground-page: #fcfcfd;/);
  assert.match(light, /--ground-page: #fcfcfd;/);
  assert.match(sheet, /--type-display: 400 48px\/56px var\(--font-sans\);/);
});

test('type on screen can only be one of the eight steps', () => {
  assert.equal(Object.keys(tokens.type.scale).length, 8);
  assert.deepEqual(messages('.a { font: var(--type-body); letter-spacing: var(--type-body-tracking); font-weight: var(--weight-medium); }'), []);
  assert.deepEqual(messages('.a { font-size: 13px; }'), ['font-size uses a raw size (13px)', 'font-size uses type that is not a step of the scale']);
  assert.deepEqual(messages('.a { line-height: 1.4; }'), ['line-height uses type that is not a step of the scale']);
  assert.deepEqual(messages('.a { font-weight: 700; }'), ['font-weight uses type that is not a step of the scale']);
  assert.deepEqual(messages('.a { font: 500 var(--space-4) sans-serif; }'), ['font uses type that is not a step of the scale']);
});

test('a raw colour is refused in every form', () => {
  for (const value of ['#fff', '#1f1f1f', '#1f1f1f80', 'rgb(0 0 0 / 0.5)', 'rgba(0,0,0,.5)', 'hsl(20 60% 50%)', 'oklch(0.7 0.1 80)', 'color(display-p3 1 0 0)']) {
    assert.deepEqual(valueProblems('background', value), ['a raw colour'], value);
  }
  for (const value of ['white', 'Black', 'rebeccapurple', '1px solid red']) {
    assert.ok(valueProblems('border', value).includes('a named colour'), value);
  }
  for (const value of ['transparent', 'currentColor', 'none', 'inherit', 'var(--ground-page)', 'color-mix(in srgb, var(--fund) var(--tint-card), var(--ground-raised))']) {
    assert.deepEqual(valueProblems('background', value), [], value);
  }
});

test('a raw size or duration is refused, and what is not a design value is let through', () => {
  for (const value of ['12px', '-2px', '1.5rem', '0.5em', '34ch', '10vw', 'calc(100% - 4px)', 'translateY(-6px)']) {
    assert.equal(valueProblems('margin', value).length, 1, value);
  }
  for (const value of ['150ms', '0.2s', '.3s', 'transform 260ms ease']) {
    assert.deepEqual(valueProblems('transition', value), ['a raw duration'], value);
  }
  for (const value of ['0', '0px', '100%', '50%', '1fr', 'auto', '100dvh', 'calc(var(--space-2) * -1)', 'repeat(3, minmax(0, 1fr))', 'scale(var(--press-scale))', 'rotate(1turn)', 'env(safe-area-inset-bottom)', 'calc(1lh - var(--space-2))']) {
    assert.deepEqual(valueProblems('width', value), [], value);
  }
});

test('a token that does not exist is refused, so a slip of the hand cannot hide', () => {
  assert.deepEqual(messages('.a { color: var(--text-primry); }'), ['color uses --text-primry, which no token makes']);
  assert.deepEqual(cssProblems('.a { background: var(--fund); }', new Set([...known, ...definedIn('.fund-blue { --fund: var(--fund-blue); }')]), breakpoints), []);
  assert.deepEqual(definedIn("<span style={{ '--share': percent }} />"), ['share']);
});

test('a breakpoint outside the token file is refused', () => {
  assert.deepEqual(messages('@media (min-width: 640px) { .a { color: var(--text-muted); } }'), []);
  assert.deepEqual(messages('@media (max-width: 860px) { .a { color: var(--text-muted); } }'), ['a breakpoint at 860px that is not in the token file']);
});

test('a breach is found inside a media query, after a comment and on its own line number', () => {
  const source = '/* colour: #fff is fine in a comment */\n.a {\n  color: var(--text-muted);\n}\n@media (min-width: 640px) {\n  a:hover {\n    color: #fff;\n  }\n}\n';
  assert.deepEqual(cssProblems(source, known, breakpoints), [[7, 'color uses a raw colour']]);
});

test('markup may not carry raw values either', () => {
  assert.deepEqual(scriptProblems("const a = <i style={{ color: '#ff0000' }} />;"), [[1, 'uses a raw colour']]);
  assert.deepEqual(scriptProblems("const a = <i style={{ width: '12px' }} />;"), [[1, 'uses a raw size']]);
  assert.deepEqual(scriptProblems("el.style.transitionDuration = '150ms';"), [[1, 'uses a raw duration']]);
  assert.deepEqual(scriptProblems("const a = <a href=\"#main\" strokeWidth=\"1.75\" viewBox=\"0 0 20 20\" />; // was 12px"), []);
});

test('contrast is the standard formula', () => {
  assert.equal(contrast(parseColour('#000000'), parseColour('#ffffff')).toFixed(2), '21.00');
  assert.equal(contrast(parseColour('#777777'), parseColour('#ffffff')).toFixed(2), '4.48');
  // A see-through colour is judged as drawn on its ground.
  assert.equal(contrastOn('rgba(255,255,255,0.52)', '#333333').toFixed(2), '4.66');
  assert.throws(() => parseColour('gold'));
});

test('every pair in the token file passes, and a colour that slips is caught', () => {
  assert.deepEqual(problems(), []);
  const dimmed = structuredClone(tokens);
  dimmed.color.dark.text.faint = 'rgba(255,255,255,0.38)';
  assert.match(problems(dimmed).join('\n'), /dark: text\.faint on every ground: the table tests rgba\(255,255,255,0\.52\) but the colour is now rgba\(255,255,255,0\.38\)/);
  const failing = structuredClone(tokens);
  failing.color.dark.text.faint = failing.contrast.dark[2].token = 'rgba(255,255,255,0.38)';
  assert.match(problems(failing).join('\n'), /dark: text\.faint on every ground on hover: 3\.24 is under 4\.5/);
  const tinted = structuredClone(tokens);
  tinted.elevation.cardTintPercent = 60;
  assert.match(problems(tinted).join('\n'), /on a card washed with fund\./);
});

test('the budget counts what the page itself loads', () => {
  const html = `<head><script type="module" crossorigin src="/assets/index-a.js"></script><link rel="modulepreload" href="/assets/vendor-b.js"><link rel="stylesheet" href="/assets/index-c.css"><link rel="preload" as="font" href="/assets/f.woff2"><link rel="icon" href="/icon.svg"></head><body><script type="application/json" id="seed">{}</script></body>`;
  assert.deepEqual(loadedBy(html), { scripts: ['/assets/index-a.js', '/assets/vendor-b.js'], styles: ['/assets/index-c.css'], fonts: ['/assets/f.woff2'] });
  assert.equal(isOurs('/assets/index-a.js'), true);
  assert.equal(isOurs('https://elsewhere.example/a.js'), false);
  assert.equal(isOurs('//elsewhere.example/a.js'), false);
  assert.equal(isOurs(undefined), false);
});

test('over budget is said plainly, and on budget says nothing', () => {
  const kb = 1024;
  assert.deepEqual(overBudget(tokens.budget, { script: 120 * kb, css: 15 * kb, fonts: 1, outside: 0 }), []);
  assert.deepEqual(overBudget(tokens.budget, { script: 120 * kb + 1, css: 16 * kb, fonts: 2, outside: 1 }), [
    'script is 120.0 KB, over 120 KB',
    'CSS is 16.0 KB, over 15 KB',
    '2 font files are fetched early, not 1',
    '1 scripts or sheets come from outside the build',
  ]);
});

test('one font file means one', () => {
  const sheet = "@font-face{font-family:A;src:url(/assets/latin.woff2) format('woff2-variations');unicode-range:U+0000-00FF,U+2026,U+2212}@font-face{font-family:A;src:url(/assets/greek.woff2);unicode-range:U+0370-03FF}";
  const faces = fontFaces(sheet);
  assert.deepEqual(faces, [
    { url: '/assets/latin.woff2', ranges: [[0, 0xff], [0x2026, 0x2026], [0x2212, 0x2212]] },
    { url: '/assets/greek.woff2', ranges: [[0x370, 0x3ff]] },
  ]);
  assert.deepEqual(extraFonts(faces, '/assets/latin.woff2', 'Caf\u00e9 \u2026 \u22121'), []);
  assert.deepEqual(extraFonts(faces, '/assets/latin.woff2', 'A \u2192 B'), ['the words of the app use \u2192 (U+2192), which the one font file does not hold']);
  assert.deepEqual(extraFonts(fontFaces('@font-face{src:url(/assets/latin.woff2);unicode-range:U+0-7F}@font-face{src:url(/assets/all.woff2)}'), '/assets/latin.woff2', ''), ['/assets/all.woff2 has no unicode-range, so every page would fetch it']);
  assert.deepEqual(extraFonts(faces, '/assets/other.woff2', ''), ['no font face uses /assets/other.woff2, the file fetched early']);
});
