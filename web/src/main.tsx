import '@fontsource-variable/inter-tight';
import './tokens.css';
import './base.css';
import './parts/parts.css';
import './screens/screens.css';
import { StrictMode } from 'react';
import { createRoot, hydrateRoot } from 'react-dom/client';
import { seed } from './api.ts';
import { App } from './App.tsx';
import { forwardOldAddress } from './route.ts';

/** What the server put in the page beside the words: its answers, and whether it failed to make the page. */
function sent(): { resources: Record<string, unknown>; broke?: boolean } | null {
  try {
    const page = JSON.parse(document.getElementById('seed')?.textContent ?? 'null');
    return typeof page?.resources === 'object' && page.resources !== null ? page : null;
  } catch {
    return null;
  }
}

const root = document.getElementById('root')!;
const page = sent();
if (page) seed(page.resources);

// An address from before real paths is moved to its new home before anything is drawn,
// and again if one turns up later, from a link or the history list.
const forwarded = forwardOldAddress();
window.addEventListener('hashchange', forwardOldAddress);

const app = (
  <StrictMode>
    <App broke={page?.broke} />
  </StrictMode>
);
// The server's words are taken over as they stand. Where it sent none, or sent
// those of another address, the page is drawn from nothing.
if (page && !forwarded) hydrateRoot(root, app);
else createRoot(root).render(app);
