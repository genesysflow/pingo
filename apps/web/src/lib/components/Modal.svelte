<script lang="ts">
  import { onMount, type Snippet } from 'svelte';
  import Icon from './Icon.svelte';
  let {
    title,
    close,
    children,
    wide = false,
  }: { title: string; close: () => void; children: Snippet; wide?: boolean } = $props();
  let element: HTMLDivElement;
  onMount(() => {
    const previous = document.activeElement as HTMLElement | null;
    element.querySelector<HTMLElement>('[autofocus], input, button, textarea, select')?.focus();
    return () => previous?.focus();
  });
  function keydown(e: KeyboardEvent) {
    if (e.key === 'Escape') {
      e.preventDefault();
      e.stopPropagation();
      close();
    }
    if (e.key === 'Tab') {
      const nodes = [
        ...element.querySelectorAll<HTMLElement>(
          'button:not(:disabled), input:not(:disabled), textarea, select, a[href], [tabindex="0"]',
        ),
      ].filter((n) => n.getClientRects().length);
      const first = nodes[0],
        last = nodes.at(-1);
      if (e.shiftKey && document.activeElement === first) {
        e.preventDefault();
        last?.focus();
      } else if (!e.shiftKey && document.activeElement === last) {
        e.preventDefault();
        first?.focus();
      }
    }
  }
</script>

<div
  class="modal-backdrop"
  role="presentation"
  onclick={(e) => {
    if (e.target === e.currentTarget) close();
  }}
>
  <div
    class="modal"
    class:wide
    role="dialog"
    aria-modal="true"
    aria-label={title}
    tabindex="-1"
    bind:this={element}
    onkeydown={keydown}
  >
    <div class="modal-title">
      <h2>{title}</h2>
      <button class="icon-button" onclick={close} aria-label="Close dialog"><Icon name="close" /></button>
    </div>
    {@render children()}
  </div>
</div>
