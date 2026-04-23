<script lang="ts">
  import "$lib/styles/global.css";
  import { onMount } from "svelte";
  let { children } = $props();

  // Suppress the WebView2 default right-click menu (Print, Save as, Import
  // passwords, More tools, …) globally. Editable fields keep theirs so
  // Copy/Cut/Paste/Select All still work on text inputs. Custom per-element
  // menus (e.g. preset rows) add their own `oncontextmenu` handlers and
  // remain unaffected — preventDefault here is idempotent.
  onMount(() => {
    const handler = (e: MouseEvent) => {
      const t = e.target as HTMLElement | null;
      if (!t) return;
      const editable =
        t.tagName === "INPUT" ||
        t.tagName === "TEXTAREA" ||
        t.isContentEditable;
      if (!editable) e.preventDefault();
    };
    window.addEventListener("contextmenu", handler);
    return () => window.removeEventListener("contextmenu", handler);
  });
</script>

{@render children()}
