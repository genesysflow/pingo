<script lang="ts">
  import { onMount } from 'svelte';
  import {
    checkServer,
    incompatibility,
    loadDesktopPreferences,
    normalizeServer,
    saveDesktopPreferences,
  } from './bridge';
  import { updates } from './updates.svelte';
  let { inCall }: { inCall: boolean } = $props();
  let server = $state(''),
    shortcut = $state(''),
    error = $state(''),
    saved = $state(''),
    updateError = $state('');
  onMount(() => {
    updates.start();
    void loadDesktopPreferences().then((p) => {
      server = p.serverUrl ?? '';
      shortcut = p.pushToTalkShortcut ?? '';
    });
  });
  async function save() {
    error = '';
    saved = '';
    try {
      const old = await loadDesktopPreferences();
      const origin = normalizeServer(server);
      if (origin !== old.serverUrl) {
        const problem = incompatibility(await checkServer(origin));
        if (problem) throw new Error(problem);
      }
      await saveDesktopPreferences({ serverUrl: origin, pushToTalkShortcut: shortcut || null });
      if (old.serverUrl !== origin) location.href = '/';
      else saved = 'Desktop settings saved.';
    } catch (e) {
      error = e instanceof Error ? e.message : String(e);
    }
  }
  async function restart() {
    updateError = '';
    try {
      await updates.install();
    } catch (e) {
      updateError = String(e);
    }
  }
  const status = $derived(updates.status);
</script>

<h3>Updates</h3>
<p class="settings-description">
  Pingo {updates.appVersion}.
  {#if status.state === 'disabled'}Development builds don’t update themselves.
  {:else if status.state === 'checking'}Checking for updates…
  {:else if status.state === 'upToDate'}You have the latest version.
  {:else if status.state === 'downloading'}Downloading {status.version}…
  {:else if status.state === 'ready'}Version {status.version} is downloaded and installs when Pingo restarts.
  {:else if status.state === 'failed'}The last update check failed: {status.message}
  {:else}Pingo checks for updates in the background.{/if}
</p>
{#if status.state === 'ready'}{#if status.notes}<p class="update-notes">{status.notes}</p>{/if}<button
    class="button primary"
    disabled={inCall}
    onclick={restart}>{inCall ? 'Restart after your call' : 'Restart to update'}</button
  >{:else}<button
    class="button"
    disabled={['disabled', 'checking', 'downloading'].includes(status.state)}
    onclick={() => updates.check()}>Check for updates</button
  >{/if}
{#if updateError}<p class="error" role="alert">{updateError}</p>{/if}

<h3>Server and shortcuts</h3>
<label class="field"
  >Server address<input bind:value={server} inputmode="url" autocapitalize="off" spellcheck="false" /></label
>
<label class="field"
  >Global push-to-talk shortcut<input
    bind:value={shortcut}
    placeholder="CommandOrControl+Shift+Space"
  /></label
>
<p class="settings-description">
  Changing the server signs you in there instead. Global shortcuts may be unavailable on Wayland. In a call,
  enable push-to-talk and hold Space while Pingo is focused. Clear the shortcut to disable it.
</p>
<button class="button" onclick={save}>Save desktop settings</button>
{#if error}<p class="error" role="alert">{error}</p>{/if}
{#if saved}<p role="status">{saved}</p>{/if}
