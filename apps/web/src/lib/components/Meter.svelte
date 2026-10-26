<script lang="ts">
  /** A segmented level meter; `level` is 0–1. Top segments warm up like a console VU. */
  let { level, segments = 6, label }: { level: number; segments?: number; label?: string } = $props();
  const lit = $derived(Math.round(Math.max(0, Math.min(1, level)) * segments));
</script>

<span class="meter" role={label ? 'meter' : undefined} aria-label={label} aria-valuemin={label ? 0 : undefined} aria-valuemax={label ? 100 : undefined} aria-valuenow={label ? Math.round(level * 100) : undefined} aria-hidden={label ? undefined : 'true'}
  >{#each Array(segments) as _, i}<i class:lit={i < lit} class:hot={i >= segments - 1} class:warm={i >= segments * 0.6 && i < segments - 1}></i>{/each}</span
>
