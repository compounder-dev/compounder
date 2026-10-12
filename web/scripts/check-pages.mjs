// The server writes the first screen's words itself, in its own language, and the
// parts take them over in the browser. Both must draw the same markup. This draws
// each case in contract/cases.json with the parts and compares it with the page kept
// beside it. The server's tests compare its own pages with the same files.
//
// After a change to a part or a screen, `node scripts/check-pages.mjs --write`
// rewrites the kept pages. The server's tests then say what it has to match.

import { readFileSync, writeFileSync } from 'node:fs';
import { fileURLToPath } from 'node:url';
import { createServer } from 'vite';

const root = fileURLToPath(new URL('..', import.meta.url));
const kept = (name) => new URL(`../contract/${name}.html`, import.meta.url);
const read = (url) => {
  try {
    return readFileSync(url, 'utf8');
  } catch {
    return null;
  }
};

/** Where two texts first part, with a little of each around it. */
function firstDifference(a, b) {
  let at = 0;
  while (at < a.length && at < b.length && a[at] === b[at]) at += 1;
  const around = (text) => JSON.stringify(text.slice(Math.max(0, at - 60), at + 60));
  return `at character ${at}\n    kept:  ${around(a)}\n    drawn: ${around(b)}`;
}

const write = process.argv.includes('--write');
const cases = JSON.parse(readFileSync(new URL('../contract/cases.json', import.meta.url), 'utf8'));
// The parts are loaded straight from source, with no second build.
const vite = await createServer({ root, logLevel: 'error', appType: 'custom', server: { middlewareMode: true, hmr: false, watch: null }, optimizeDeps: { noDiscovery: true } });
const failures = [];
try {
  const { page } = await vite.ssrLoadModule('/src/contract.tsx');
  for (const each of cases) {
    const drawn = page(each.address, each.resources, each.broke ?? false);
    // Drawn outside a browser, the join between two pieces of text is marked with a comment. The
    // server writes plain markup and cannot, so a part must hand over each run of words as one piece.
    if (drawn.includes('<!--')) failures.push(`${each.name}: the markup has a join the server cannot write. Make the words around it one string.\n    ${JSON.stringify(drawn.slice(Math.max(0, drawn.indexOf('<!--') - 80), drawn.indexOf('<!--') + 40))}`);
    else if (write) writeFileSync(kept(each.name), `${drawn}\n`);
    else if (read(kept(each.name)) !== `${drawn}\n`) failures.push(`${each.name}: contract/${each.name}.html is not what the parts draw, ${read(kept(each.name)) === null ? 'the file is missing' : firstDifference(read(kept(each.name)), `${drawn}\n`)}`);
  }
} finally {
  await vite.close();
}

if (failures.length) {
  console.error(`pages: ${failures.length} of ${cases.length} differ\n  ${failures.join('\n  ')}\n  If the change is meant, run: node scripts/check-pages.mjs --write`);
  process.exit(1);
}
console.log(`pages: ${cases.length} ${write ? 'written to contract/' : 'match what the server is held to'}`);
