import { defineConfig } from 'vite';

// In development the Rust backend serves the API on 8787.
export default defineConfig({
  server: { proxy: { '/v1': 'http://127.0.0.1:8787', '/health': 'http://127.0.0.1:8787' } },
});
