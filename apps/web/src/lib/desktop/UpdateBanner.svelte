<script lang="ts">
  import { onMount } from 'svelte';
  import Icon from '../components/Icon.svelte';
  import { updates } from './updates.svelte';
  let { inCall, notice }: { inCall: boolean; notice: (text: string) => void } = $props();
  let dismissed = $state(''),
    busy = $state(false);
  const ready = $derived(updates.status.state === 'ready' ? updates.status : null);
  onMount(() => updates.start());
  async function restart() {
    busy = true;
    try {
      await updates.install();
    } catch (e) {
      notice(String(e));
    } finally {
      busy = false;
    }
  }
</script>

{#if ready && dismissed !== ready.version}<div class="update-card" role="status">
    <Icon name="download" size={16} />
    <div>
      <strong>Pingo {ready.version} is ready</strong>
      <span>{inCall ? 'Restart after your call to update.' : 'Restart to finish updating.'}</span>
    </div>
    <button class="button primary small" disabled={inCall || busy} onclick={restart}>Restart</button>
    <button class="icon-button" aria-label="Later" title="Later" onclick={() => (dismissed = ready.version)}
      ><Icon name="close" size={14} /></button
    >
  </div>{/if}
