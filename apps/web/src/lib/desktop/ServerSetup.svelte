<script lang="ts">
  import { loadDesktopPreferences, saveDesktopPreferences } from './bridge';
  let { ready }: { ready: () => void } = $props();
  let server = $state('http://127.0.0.1:8090'), error = $state(''), busy = $state(false);
  async function connect(event: SubmitEvent) {
    event.preventDefault();
    busy = true;
    error = '';
    try {
      const current = await loadDesktopPreferences();
      await saveDesktopPreferences({ ...current, serverUrl: server });
      ready();
    } catch (e) { error = String(e); }
    finally { busy = false; }
  }
</script>

<main class="setup">
  <form onsubmit={connect}>
    <div class="eyebrow">PINGO</div>
    <h1>Find your people.</h1>
    <p>Enter your Pingo server address. We’ll remember it on this device.</p>
    <label class="field">Server URL<input type="url" bind:value={server} required placeholder="https://pingo.example.com" /></label>
    {#if error}<p role="alert" class="error">{error}</p>{/if}
    <button class="button primary" disabled={busy}>{busy ? 'Connecting…' : 'Continue'}</button>
  </form>
</main>

<style>
  .setup { min-height: 100vh; display: grid; place-items: center; padding: 2rem; }
  form { width: min(100%, 440px); }
  p { margin: 1rem 0 2rem; }
</style>
