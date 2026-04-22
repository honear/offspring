// Tauri doesn't have a Node.js server. Every route in `src/routes/` is
// prerendered via `export const prerender = true` in `+layout.ts`, so we get
// a real HTML file per route (`build/index.html`, `build/progress.html`,
// `build/custom.html`) that Tauri's asset server serves directly.
//
// We deliberately do NOT set a `fallback` — adapter-static would write the
// fallback page to `build/index.html` AFTER the prerendered root, overwriting
// it and breaking the main window (loads a bare SvelteKit shell that 404s the
// `/` route on client-side hydration). We have no "unknown route" case inside
// a Tauri app anyway: each window loads a specific prerendered file.
// See: https://v2.tauri.app/start/frontend/sveltekit/ for more info
import adapter from "@sveltejs/adapter-static";
import { vitePreprocess } from "@sveltejs/vite-plugin-svelte";

/** @type {import('@sveltejs/kit').Config} */
const config = {
  preprocess: vitePreprocess(),
  kit: {
    adapter: adapter(),
  },
};

export default config;
