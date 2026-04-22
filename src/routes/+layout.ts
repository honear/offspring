// Tauri doesn't have a Node.js server, so we disable SSR and let SvelteKit
// prerender a static HTML shell per route. Every route then hydrates
// client-side (SPA-style) once loaded.
//
// We *must* prerender (not rely on SPA fallback alone) because Tauri opens
// multiple webview windows — /progress and /custom — as separate top-level
// URLs. Asset-server fallback to index.html works flakily for the second
// window in an instance (the Custom→Encode handoff opens the progress window
// as a blank page), so we make each route exist as a real file on disk.
// See: https://svelte.dev/docs/kit/single-page-apps
// See: https://v2.tauri.app/start/frontend/sveltekit/ for more info
export const ssr = false;
export const prerender = true;

// Emit each route as `build/<route>/index.html` (folder layout) rather than
// flat `build/<route>.html`. With the flat layout, Tauri loads the window
// at URL `/progress.html`, and SvelteKit's client router then tries to match
// the pathname against its routes — there is no `/progress.html` route
// (only `/progress`), so the router immediately 404s. With trailing-slash
// URLs (`/progress/`) the router normalizes and matches `/progress`.
export const trailingSlash = "always";
