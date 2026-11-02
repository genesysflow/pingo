<script lang="ts">
  import { onMount, untrack } from 'svelte';
  import {
    checkServer,
    incompatibility,
    loadDesktopPreferences,
    normalizeServer,
    saveDesktopPreferences,
  } from './bridge';
  import { updates } from './updates.svelte';
  import AuthStory from '../components/AuthStory.svelte';
  import { DEFAULT_ACCENT, inkFor, storedTheme } from '../theme';
  let {
    current,
    problem = '',
    ready,
  }: {
    current: string | null;
    /** Why the saved server can't be used, when the startup check found it incompatible. */
    problem?: string;
    ready: () => void;
  } = $props();
  // Development builds point at the local dev server so a fresh checkout works without typing.
  let server = $state(untrack(() => current) ?? (import.meta.env.DEV ? 'http://127.0.0.1:8090' : '')),
    error = $state(untrack(() => problem)),
    busy = $state(false);
  onMount(() => updates.start());
  const update = $derived(updates.status);
  async function connect(event: SubmitEvent) {
    event.preventDefault();
    busy = true;
    error = '';
    try {
      const origin = normalizeServer(server);
      const problem = incompatibility(await checkServer(origin));
      if (problem) throw new Error(problem);
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
        <h2>{problem ? 'Can’t use this server' : current ? 'Change server' : 'Connect to a server'}</h2>
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
        {#if update.state === 'ready'}<button
            type="button"
            class="button full update-action"
            onclick={() => updates.install().catch((e) => (error = String(e)))}
            >Restart to update to Pingo {update.version}</button
          >{:else if update.state === 'downloading'}<p class="muted tiny">
            Downloading Pingo {update.version}…
          </p>{/if}
        {#if current && !problem}<button type="button" class="magic-link" onclick={ready}
            >Keep using {new URL(current).host}</button
          >{/if}
      </form>
    </main>
  </div>
</div>
