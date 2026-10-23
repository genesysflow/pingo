<script lang="ts">
  import { marked } from 'marked';
  import DOMPurify from 'dompurify';
  let { text }: { text: string } = $props();
  const html = $derived(DOMPurify.sanitize(marked.parse(text, { async: false, breaks: true }), { FORBID_TAGS: ['style', 'iframe'], FORBID_ATTR: ['style'] }));
</script>
<div class="markdown">{@html html}</div>
