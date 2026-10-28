import { defineConfig } from 'vite';
import { svelte } from '@sveltejs/vite-plugin-svelte';
import { readFileSync } from 'node:fs';

const version = readFileSync('../backend/Cargo.toml', 'utf8').match(/^version\s*=\s*"([^"]+)"/m)[1];

export default defineConfig({
  plugins: [
    svelte(),
    {
      // Fonts are only discovered after the CSS parses; preload them so text paints with the page.
      name: 'preload-fonts',
      transformIndexHtml: (_, { bundle }) =>
        Object.keys(bundle ?? {})
          .filter((f) => /latin-wght-normal.*\.woff2$/.test(f))
          .map((f) => ({ tag: 'link', attrs: { rel: 'preload', as: 'font', type: 'font/woff2', crossorigin: '', href: '/' + f }, injectTo: 'head' })),
    },
  ],
  define: { __APP_VERSION__: JSON.stringify(version) },
  clearScreen: false,
  server: { port: 1420, strictPort: true },
  build: { target: 'chrome130' },
});
