import { defineConfig, type HtmlTagDescriptor, type Plugin } from 'vite';
import tokens from './tokens.json';

/**
 * Puts into the page what only the build knows: the page colour of each theme, for the
 * browser's own frame, and the one font file to fetch before the stylesheet asks for it.
 */
function pageHead(): Plugin {
  return {
    name: 'page-head',
    transformIndexHtml: {
      order: 'post',
      handler(_html, context) {
        const tags: HtmlTagDescriptor[] = (['dark', 'light'] as const).map((theme) => ({
          tag: 'meta',
          attrs: { name: 'theme-color', media: `(prefers-color-scheme: ${theme})`, content: tokens.color[theme].ground.page },
          injectTo: 'head',
        }));
        const font = Object.keys(context.bundle ?? {}).find((file) => /inter-tight-latin-wght-normal-[\w-]+\.woff2$/.test(file));
        if (font) tags.push({ tag: 'link', attrs: { rel: 'preload', as: 'font', type: 'font/woff2', href: `/${font}`, crossorigin: '' }, injectTo: 'head' });
        return tags;
      },
    },
  };
}

// In development the Rust backend serves the API on 8787.
export default defineConfig({
  plugins: [pageHead()],
  server: { proxy: { '/v1': 'http://127.0.0.1:8787', '/health': 'http://127.0.0.1:8787' } },
});
