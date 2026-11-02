<script lang="ts">
  import { untrack } from 'svelte';
  import { checkServer, loadDesktopPreferences, normalizeServer, saveDesktopPreferences } from './bridge';
  import AuthStory from '../components/AuthStory.svelte';
  import { DEFAULT_ACCENT, inkFor, storedTheme } from '../theme';
  let { current, ready }: { current: string | null; ready: () => void } = $props();
  // Development builds point at the local dev server so a fresh checkout works without typing.
  let server = $state(untrack(() => current) ?? (import.meta.env.DEV ? 'http://127.0.0.1:8090' : '')),
    error = $state(''),
    busy = $state(false);
  async function connect(event: SubmitEvent) {
    event.preventDefault();
    busy = true;
    error = '';
    try {
      const origin = normalizeServer(server);
      await checkServer(origin);
      const preferences = await loadDesktopPreferences();
      await saveDesktopPreferences({ ...preferences, serverUrl: origin });
      ready();
    } catch (e) {
      error = e instanceof Error ? e.message : String(e);
    } finally {
      busy = false;
    }
  }
</script>

<div
  class="app-root"
  data-theme={storedTheme()}
  style:--accent={DEFAULT_ACCENT}
  style:--on-accent={inkFor(DEFAULT_ACCENT)}
>
  <div class="auth-page">
    <AuthStory />
    <main class="auth-form-side">
      <form class="auth-form" onsubmit={connect}>
        <h2>{current ? 'Change server' : 'Connect to a server'}</h2>
        <p class="settings-description">
          Enter the address of the Pingo server you want to use. This device will remember it.
        </p>
        <label class="field"
          >Server address<input
            bind:value={server}
            required
            inputmode="url"
            autocomplete="url"
            autocapitalize="off"
            spellcheck="false"
            placeholder="pingo.example.com"
          /></label
        >
        {#if error}<p role="alert" class="error">{error}</p>{/if}
        <button class="button primary full" disabled={busy || !server.trim()}
          >{busy ? 'Connecting…' : 'Continue'}</button
        >
        {#if current}<button type="button" class="magic-link" onclick={ready}
            >Keep using {new URL(current).host}</button
          >{/if}
      </form>
    </main>
  </div>
</div>
