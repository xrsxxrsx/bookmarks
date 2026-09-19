import { defineConfig } from 'vite';
import react from '@vitejs/plugin-react';

// `dist` is what tauri.conf.json points `frontendDist` at, so `npm run build` produces
// exactly what the desktop shell loads.
export default defineConfig({
  plugins: [react()],
  // Tauri serves the dev server on a fixed port and expects a clear screen.
  clearScreen: false,
  server: {
    port: 5173,
    strictPort: true,
    watch: {
      /*
       * Editors that save atomically create a temporary directory next to the target file, then
       * rename over it. Vite's watcher tries to watch that directory, gets EBUSY because the file
       * is already gone, and — because a watcher error is unhandled — the whole dev server exits.
       * The window is then left pointing at a dead port.
       *
       * Ignoring those paths keeps the server alive. They are never source files, so nothing is
       * lost by not watching them.
       */
      ignored: ['**/.*.tmpdir/**', '**/.*.tmpdir', '**/*.tmp', '**/.~*'],
    },
  },
  build: {
    outDir: 'dist',
    emptyOutDir: true,
    // WebView2 is a known-modern target, so no legacy downlevelling is needed.
    target: 'chrome110',
    sourcemap: false,
  },
});
