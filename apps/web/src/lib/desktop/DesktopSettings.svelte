<script lang="ts">
  import { onMount } from 'svelte';
  import { loadDesktopPreferences, saveDesktopPreferences } from './bridge';
  let server = $state(''), shortcut = $state(''), error = $state(''), saved = $state('');
  onMount(() => { void loadDesktopPreferences().then(p => { server = p.serverUrl ?? ''; shortcut = p.pushToTalkShortcut ?? ''; }); });
  async function save() {
    error = ''; saved = '';
    try {
      const old = await loadDesktopPreferences();
      await saveDesktopPreferences({ serverUrl: server, pushToTalkShortcut: shortcut || null });
      if (old.serverUrl !== server) location.href = '/';
      else saved = 'Desktop settings saved.';
    } catch (e) { error = String(e); }
  }
</script>

<h3>Desktop</h3>
<label class="field">Server URL<input type="url" bind:value={server} /></label>
<label class="field">Global push-to-talk shortcut<input bind:value={shortcut} placeholder="CommandOrControl+Shift+Space" /></label>
<p class="settings-description">Global shortcuts may be unavailable on Wayland. In a call, enable push-to-talk and hold Space while Pingo is focused. Clear the shortcut to disable it.</p>
<button class="button" onclick={save}>Save desktop settings</button>
{#if error}<p class="error" role="alert">{error}</p>{/if}
{#if saved}<p role="status">{saved}</p>{/if}
