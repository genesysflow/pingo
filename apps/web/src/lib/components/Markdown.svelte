<script lang="ts">
  import { renderMarkdown } from '../logic/markdown';
  let { text }: { text: string } = $props();
  const html = $derived(renderMarkdown(text));
</script>

<div class="markdown">{@html html}</div>
