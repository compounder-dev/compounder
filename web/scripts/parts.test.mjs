// What each part puts in the page, for people who see it and people who hear it.
// The parts are loaded from source and drawn as markup. What needs a real browser,
// such as focus inside a sheet, is checked by hand on the parts page.

import assert from 'node:assert/strict';
import { after, before, test } from 'node:test';
import { fileURLToPath } from 'node:url';
import { createElement as h } from 'react';
import { renderToStaticMarkup } from 'react-dom/server';
import { createServer } from 'vite';

let vite;
let parts;
const draw = (element) => renderToStaticMarkup(element);

before(async () => {
  vite = await createServer({ root: fileURLToPath(new URL('..', import.meta.url)), logLevel: 'error', appType: 'custom', server: { middlewareMode: true, hmr: false, watch: null }, optimizeDeps: { noDiscovery: true } });
  parts = await vite.ssrLoadModule('/src/parts/index.ts');
});

after(() => vite.close());

test('a button is a real button and never submits a form by accident', () => {
  assert.equal(draw(h(parts.Button, null, 'Go')), '<button type="button" class="button button-secondary">Go</button>');
  assert.equal(draw(h(parts.Button, { variant: 'primary', size: 'lg', fullWidth: true }, 'Go')), '<button type="button" class="button button-primary button-lg button-full">Go</button>');
  assert.match(draw(h(parts.Button, { type: 'submit' }, 'Go')), /type="submit"/);
  for (const variant of ['primary', 'secondary', 'ghost', 'danger']) assert.match(draw(h(parts.Button, { variant }, 'Go')), new RegExp(`class="button button-${variant}"`));
});

test('a loading button keeps its words, says it is busy and stays where focus can find it', () => {
  const markup = draw(h(parts.Button, { loading: true }, 'Saving'));
  assert.match(markup, /aria-busy="true"/);
  assert.match(markup, /aria-disabled="true"/);
  assert.doesNotMatch(markup, / disabled/);
  assert.match(markup, /<span class="spinner" aria-hidden="true"><\/span>Saving<\/button>$/);
  assert.match(draw(h(parts.Button, { disabled: true }, 'Off')), /<button disabled="" type="button"/);
});

test('an icon button always has a spoken label, and its icon is not read', () => {
  const markup = draw(h(parts.IconButton, { icon: 'close', label: 'Close' }));
  assert.match(markup, /^<button type="button" class="icon-button icon-button-ghost" aria-label="Close"><svg class="icon icon-md"[^>]*aria-hidden="true">/);
  assert.match(draw(h(parts.IconButton, { icon: 'close', label: 'Close', variant: 'filled' })), /icon-button-filled/);
});

test('a link is a real link in both variants', () => {
  assert.equal(draw(h(parts.Link, { href: '/s/a' }, 'Open')), '<a class="link link-inline" href="/s/a">Open</a>');
  assert.match(draw(h(parts.Link, { href: '/', variant: 'standalone', icon: 'arrow-left' }, 'Back')), /^<a class="link link-standalone" href="\/"><svg/);
  assert.equal(draw(h(parts.ButtonLink, { href: '/', variant: 'primary' }, 'Go home')), '<a class="button button-primary" href="/">Go home</a>');
});

test('a card that opens something has one link, in its title', () => {
  const markup = draw(h(parts.Card, { variant: 'strategy', fund: 'teal', as: 'li' }, h(parts.CardTitle, { href: '/s/a', dot: true }, 'Name')));
  assert.equal(markup, '<li class="card card-strategy fund-teal"><h2 class="card-title"><span class="fund-dot" aria-hidden="true"></span><a class="card-link" href="/s/a">Name</a></h2></li>');
  assert.equal(markup.match(/<a /g).length, 1);
  assert.equal(draw(h(parts.Card, { label: 'Facts', as: 'aside' }, 'x')), '<aside class="card card-plain" aria-label="Facts">x</aside>');
});

test('a row is plain, a link, or a button that says whether it is open', () => {
  assert.equal(draw(h(parts.Row, { title: 'Gold', detail: 'GLD', trailing: '40%' })), '<li class="row row-asset"><span class="row-text"><span class="row-title">Gold</span><span class="row-detail">GLD</span></span><span class="row-trailing">40%</span></li>');
  assert.match(draw(h(parts.Row, { title: 'Fund', href: '/f/1', variant: 'fund' })), /^<li class="row-slot"><a class="row row-fund row-action" href="\/f\/1">.*<svg/);
  assert.match(draw(h(parts.Row, { title: 'Setting', variant: 'setting', onToggle() {} })), /<button type="button" class="row row-setting row-action" aria-expanded="false">/);
  assert.match(draw(h(parts.Row, { title: 'Setting', open: true, onToggle() {} })), /aria-expanded="true"/);
  assert.equal(draw(h(parts.Mark, { letter: 'G' })), '<span class="mark" aria-hidden="true">G</span>');
  assert.equal(draw(h(parts.WeightBar, { percent: 37 })), '<span class="bar" aria-hidden="true"><span style="--share:37"></span></span>');
});

test('a number is one piece of text, so it is read as a number', () => {
  assert.equal(draw(h(parts.Number, { kind: 'money', value: 1240.18 })), '<span class="number">$1,240.18</span>');
  assert.equal(draw(h(parts.Number, { kind: 'percent', value: 0.142857 })), '<span class="number">14.3%</span>');
  assert.equal(draw(h(parts.Number, { kind: 'percent', value: -0.182, tone: 'loss', sign: true, size: 'title2' })), '<span class="number number-title2 number-loss">−18.2%</span>');
  assert.equal(draw(h(parts.Number, { kind: 'percent', value: 0, sign: true, arrow: true })), '<span class="number">0%</span>');
});

test('a change carries an arrow as well as its colour, and the arrow is not read', () => {
  assert.match(draw(h(parts.Number, { kind: 'percent', value: 0.127, tone: 'gain', sign: true, arrow: true })), /^<span class="number number-gain"><svg class="icon icon-sm"[^>]*aria-hidden="true"><path d="M10 16V4[^"]*"><\/path><\/svg>\+12\.7%<\/span>$/);
  assert.match(draw(h(parts.Number, { kind: 'percent', value: -0.127, tone: 'loss', sign: true, arrow: true })), /<path d="M10 4v12/);
  // A change too small to print has no direction to point in.
  assert.equal(draw(h(parts.Number, { kind: 'percent', value: 0.00001, sign: true, arrow: true })), '<span class="number">0%</span>');
});

test('a number that changed is washed by the way it moved', async () => {
  const { movement } = await vite.ssrLoadModule('/src/parts/Number.tsx');
  assert.equal(movement(1, 2), 'up');
  assert.equal(movement(2, 1), 'down');
  assert.equal(movement(1, 1), null);
  assert.equal(movement(null, 1), null);
  assert.equal(movement(1, null), null);
});

test('a number that is not known shows a dash and says so aloud', () => {
  assert.equal(draw(h(parts.Number, { kind: 'money', value: null })), '<span class="number"><span aria-hidden="true">–</span><span class="sr-only">not available</span></span>');
  assert.equal(draw(h(parts.Number, { kind: 'money', value: NaN })), draw(h(parts.Number, { kind: 'money', value: null })));
  assert.equal(draw(h(parts.Number, { kind: 'money', value: null, loading: true })), '<span class="number skeleton skeleton-number" aria-hidden="true"></span>');
});

test('a chip is an icon and a word, in each of its six kinds', () => {
  const words = { live: 'Live', waiting: 'Waiting', paused: 'Paused', 'needs-you': 'Needs you', draft: 'Draft', 'pretend-money': 'Pretend money' };
  for (const [variant, word] of Object.entries(words)) {
    const markup = draw(h(parts.Chip, { variant }));
    assert.match(markup, new RegExp(`^<span class="chip chip-${variant}"><(svg|span) [^>]*aria-hidden="true">.*${word}</span>$`), variant);
  }
  assert.match(draw(h(parts.Chip, { variant: 'live' }, 'Live since June')), /Live since June<\/span>$/);
});

test('a skeleton is never read out and never says there is nothing', () => {
  for (const markup of [draw(h(parts.Skeleton)), draw(h(parts.CardSkeleton)), draw(h(parts.RowSkeleton))]) {
    assert.doesNotMatch(markup.replace(/<[^>]*>/g, ''), /\S/, 'a skeleton holds no words');
    for (const tag of markup.match(/<span class="[^"]*skeleton[^"]*"[^>]*>/g)) assert.match(tag, /aria-hidden="true"/);
  }
});

test('a banner speaks up by its kind: an error at once, the others when there is a pause', () => {
  assert.match(draw(h(parts.Banner, { variant: 'error' }, 'Failed.')), /^<div class="banner banner-error" role="alert"><svg[^>]*aria-hidden="true">.*<p>Failed\.<\/p><\/div>$/);
  assert.match(draw(h(parts.Banner, { variant: 'needs-you' }, 'Renew.')), /role="status"/);
  assert.match(draw(h(parts.Banner, null, 'Note.')), /^<div class="banner banner-info" role="status">/);
  assert.equal(draw(h(parts.EmptyState, { action: h(parts.Button, null, 'Look again') }, 'Nothing yet.')), '<div class="empty"><p>Nothing yet.</p><button type="button" class="button button-secondary">Look again</button></div>');
});

test('the bars mark where you are, and a badge is said as well as shown', () => {
  const items = [
    { href: '/', label: 'Home', icon: 'home', holds: (path) => path.startsWith('/s/') },
    { href: '/explore', label: 'Explore', icon: 'info', holds: () => false, badge: true },
  ];
  const onHome = draw(h(parts.BottomBar, { items, path: '/' }));
  assert.match(onHome, /^<nav class="bottom-bar" aria-label="Main"><a class="tab" href="\/" aria-current="page">/);
  assert.match(onHome, /<a class="tab" href="\/explore"><span class="tab-mark"><svg.*<span class="tab-badge" aria-hidden="true"><\/span><\/span><span>Explore<\/span><span class="sr-only">, something new<\/span><\/a>/);
  assert.match(draw(h(parts.BottomBar, { items, path: '/s/a' })), /href="\/" aria-current="true"/);
  assert.doesNotMatch(draw(h(parts.BottomBar, { items, path: '/pricing' })), /aria-current/);

  const top = draw(h(parts.TopBar, { items, path: '/explore', end: h(parts.Button, null, 'Sign in') }));
  assert.match(top, /<a class="wordmark" href="\/" aria-label="Compounder, home">/);
  assert.match(top, /<a class="top-bar-link" href="\/explore" aria-current="page">Explore<\/a>/);
  assert.match(top, /<div class="top-bar-end"><button/);
});

test('a sheet is a dialog named by its title, with a way to close it', () => {
  const markup = draw(h(parts.Sheet, { open: false, onClose() {}, title: 'Start a fund', actions: h(parts.Button, null, 'Continue') }, 'Body'));
  const id = /aria-labelledby="([^"]+)"/.exec(markup)[1];
  assert.match(markup, /^<dialog class="sheet sheet-action" data-phase="closed"/);
  assert.ok(markup.includes(`<h2 class="sheet-title" id="${id}">Start a fund</h2>`));
  assert.match(markup, /<button type="button" class="icon-button icon-button-ghost" aria-label="Close">/);
  assert.match(markup, /<div class="sheet-body">Body<\/div><footer class="sheet-actions"><button/);
  const busy = draw(h(parts.Sheet, { open: true, onClose() {}, title: 'Working', variant: 'full', busy: true }));
  assert.match(busy, /^<dialog class="sheet sheet-full" data-phase="closed" aria-labelledby="[^"]+" aria-busy="true">/);
  assert.match(busy, /<button disabled="" type="button" class="icon-button icon-button-ghost" aria-label="Close">/);
});
