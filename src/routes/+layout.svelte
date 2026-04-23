<script lang="ts">
  import "$lib/styles/global.css";
  import { onMount } from "svelte";
  let { children } = $props();

  // Unconditionally suppress the WebView2 default right-click menu (Print,
  // Save as, Import passwords, Search the web, More tools, …) and replace
  // it on editable fields with our own minimal Cut/Copy/Paste/Select All
  // menu. Non-editable elements either get their own contextmenu handler
  // (e.g. preset rows in +page.svelte) or no menu at all.
  let editCtx = $state<{
    x: number;
    y: number;
    target: HTMLInputElement | HTMLTextAreaElement;
  } | null>(null);

  onMount(() => {
    const handler = (e: MouseEvent) => {
      e.preventDefault();
      const t = e.target as HTMLElement | null;
      if (!t) {
        editCtx = null;
        return;
      }
      if (
        (t.tagName === "INPUT" && (t as HTMLInputElement).type !== "checkbox") ||
        t.tagName === "TEXTAREA"
      ) {
        editCtx = {
          x: e.clientX,
          y: e.clientY,
          target: t as HTMLInputElement | HTMLTextAreaElement,
        };
      } else {
        // A preset row (or any other non-input) already wired up its own
        // oncontextmenu handler upstream. We just swallow the default
        // WebView2 menu and leave the bespoke one alone.
        editCtx = null;
      }
    };
    const closer = () => (editCtx = null);
    window.addEventListener("contextmenu", handler);
    window.addEventListener("click", closer);
    window.addEventListener("blur", closer);
    return () => {
      window.removeEventListener("contextmenu", handler);
      window.removeEventListener("click", closer);
      window.removeEventListener("blur", closer);
    };
  });

  function runEdit(action: "cut" | "copy" | "paste" | "selectall") {
    if (!editCtx) return;
    const el = editCtx.target;
    el.focus();
    if (action === "selectall") {
      el.select();
    } else if (action === "paste") {
      // navigator.clipboard is available in WebView2 and handles text
      // pasting without the deprecation concerns of execCommand('paste').
      navigator.clipboard.readText().then((text) => {
        const start = el.selectionStart ?? el.value.length;
        const end = el.selectionEnd ?? el.value.length;
        el.setRangeText(text, start, end, "end");
        el.dispatchEvent(new Event("input", { bubbles: true }));
      });
    } else {
      // execCommand still works in WebView2 for cut/copy and preserves the
      // selection semantics that a custom implementation would have to
      // replicate (e.g. firing `input`/`change` correctly on cut).
      document.execCommand(action);
    }
    editCtx = null;
  }
</script>

{#if editCtx}
  <div
    class="edit-ctx-menu"
    style="left: {editCtx.x}px; top: {editCtx.y}px;"
    role="menu"
    onclick={(e) => e.stopPropagation()}
    onmousedown={(e) => e.preventDefault()}
  >
    <button type="button" role="menuitem" onclick={() => runEdit("cut")}>Cut</button>
    <button type="button" role="menuitem" onclick={() => runEdit("copy")}>Copy</button>
    <button type="button" role="menuitem" onclick={() => runEdit("paste")}>Paste</button>
    <div class="sep" aria-hidden="true"></div>
    <button type="button" role="menuitem" onclick={() => runEdit("selectall")}>Select all</button>
  </div>
{/if}

{@render children()}

<style>
  .edit-ctx-menu {
    position: fixed;
    z-index: 1100;
    min-width: 140px;
    padding: 4px;
    display: flex;
    flex-direction: column;
    gap: 2px;
    background: var(--c-surface);
    border: 1px solid var(--c-border);
    border-radius: var(--r-sm);
    box-shadow: 0 8px 24px rgba(0, 0, 0, 0.2);
  }
  .edit-ctx-menu button {
    all: unset;
    padding: 6px 10px;
    border-radius: var(--r-sm);
    font-size: var(--fs-13, 13px);
    cursor: pointer;
    color: var(--c-text);
  }
  .edit-ctx-menu button:hover { background: var(--c-surface-2); }
  .edit-ctx-menu .sep {
    height: 1px;
    background: var(--c-border);
    margin: 4px 2px;
  }
</style>
