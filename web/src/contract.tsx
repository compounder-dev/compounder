// The words the server must send for an address, drawn here by the parts themselves.
// scripts/check-pages.mjs compares this with the files in contract/, and the
// server's tests compare its own pages with the same files.

import { renderToString } from 'react-dom/server';
import { reset, seed } from './api.ts';
import { App } from './App.tsx';

export function page(address: string, resources: Record<string, unknown>, broke = false): string {
  reset();
  seed(resources);
  return renderToString(<App address={address} broke={broke} />);
}
