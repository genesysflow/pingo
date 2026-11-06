<script lang="ts">
  import Icon from './Icon.svelte';
  let { id, unread, pending = 0 }: { id: string; unread: number; pending?: number } = $props();
  const count = $derived(unread || pending);
  const label = $derived(
    [
      unread ? `${unread >= 100 ? '99+' : unread} unread message${unread === 1 ? '' : 's'}` : '',
      pending ? `${pending} pending inbox item${pending === 1 ? '' : 's'}` : '',
    ]
      .filter(Boolean)
      .join(', '),
  );
</script>

{#if count}
  <span class="sidebar-unread" class:attention={pending > 0} title={label}>
    <span class="sr-only" {id}>{label}</span>
    <span class="unread-number" aria-hidden="true">
      {#if !unread}<Icon name="bell" size={11} />{/if}{count >= 100 ? '99+' : count}
    </span>
  </span>
{/if}

<style>
  .sidebar-unread {
    flex: 0 0 auto;
    display: inline-flex;
    align-items: center;
    justify-content: center;
    min-width: 22px;
    height: 20px;
    padding: 0 6px;
    border-radius: 6px;
    background: var(--text);
    color: var(--sidebar);
    font: 650 11px/1 var(--font-mono);
    font-variant-numeric: tabular-nums;
  }
  .attention {
    background: var(--accent);
    color: var(--on-accent);
  }
  .unread-number {
    display: inline-flex;
    align-items: center;
    gap: 4px;
  }
</style>
