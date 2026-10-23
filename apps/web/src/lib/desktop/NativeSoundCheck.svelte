<script lang="ts">
  import { onMount, onDestroy, untrack } from 'svelte';
  import { invoke } from '@tauri-apps/api/core';
  import { listen, type UnlistenFn } from '@tauri-apps/api/event';
  import type { AudioSettings, CallController } from '../call/controller.svelte';
  import { nativeSettings } from '../call/tauri.svelte';
  import Modal from '../components/Modal.svelte';
  let { call, close, complete }: { call: CallController; close: () => void; complete: () => void } = $props();
  type Device = { deviceId: string; label: string; kind: string; isDefault: boolean };
  let devices = $state<Device[]>([]), settings = $state<AudioSettings>(untrack(() => ({ ...call.audio }))), level = $state(0), testing = $state(false), error = $state('');
  const sessionId = crypto.randomUUID();
  let stops: UnlistenFn[] = [], disposed = false;
  onMount(() => {
    void Promise.all([
      listen<Device[]>('native-devices', e => { devices = e.payload; }),
      listen<{ sessionId: string; event: { type: string; level: number; message: string } }>('native-call', e => {
        if (e.payload.sessionId !== sessionId) return;
        if (e.payload.event.type === 'loopbackLevel') level = e.payload.event.level;
        if (e.payload.event.type === 'disconnected') { testing = false; level = 0; }
        if (e.payload.event.type === 'error') error = e.payload.event.message;
      }),
    ]).then(next => { if (disposed) next.forEach(stop => stop()); else stops = next; });
    void invoke<Device[]>('call_devices').then(next => { devices = next; }).catch(e => { error = String(e); });
  });
  function stop() { testing = false; level = 0; void invoke('call_leave', { sessionId }); }
  async function test() {
    error = '';
    try { await invoke('call_loopback', { sessionId, settings: nativeSettings(settings) }); testing = true; }
    catch (e) { error = String(e); }
  }
  function finish() {
    stop(); call.configureAudio(settings); localStorage.setItem('pingo-sound-checked', 'true'); complete();
  }
  onDestroy(() => { disposed = true; stops.forEach(stop => stop()); stop(); });
</script>
+
+<Modal title="Let’s find your good sound." {close}>
+  <p class="settings-description">Choose your devices, then say a few words. You’ll hear five seconds through the same processing used in your calls. Use headphones.</p>
+  {#each [{ kind: 'audioinput', label: 'Microphone', key: 'microphone' as const }, { kind: 'audiooutput', label: 'Speaker', key: 'speaker' as const }] as choice}
+    <label class="field">{choice.label}<select disabled={testing} bind:value={settings[choice.key]}><option value="default">System default</option>{#each devices.filter(d => d.kind === choice.kind && d.deviceId !== 'default') as d}<option value={d.deviceId}>{d.label}{d.isDefault ? ' (default)' : ''}</option>{/each}</select></label>
+  {/each}
+  <label class="field">Noise suppression<select disabled={testing || settings.studio} bind:value={settings.suppression}>{#each ['Off', 'Gentle', 'Strong'] as noise}<option>{noise}</option>{/each}</select></label>
+  <label class="setting-toggle"><span><strong>Studio</strong><small>Preserve music; use headphones.</small></span><input type="checkbox" disabled={testing} bind:checked={settings.studio} /></label>
+  <label class="setting-toggle"><span><strong>Echo cancellation</strong></span><input type="checkbox" disabled={testing || settings.studio} checked={settings.echoCancel ?? true} onchange={e => { settings.echoCancel = e.currentTarget.checked; }} /></label>
+  <meter min="0" max="1" value={level} aria-label="Microphone input level"></meter>
+  <div class="inline-actions"><button class="button" disabled={!!call.room} onclick={testing ? stop : test}>{testing ? 'Stop sound check' : 'Hear myself for 5 seconds'}</button><button class="button primary" disabled={testing} onclick={finish}>Sounds good</button></div>
+  {#if call.room}<p class="settings-description">Leave your call to test your microphone.</p>{/if}
+  {#if error}<p role="alert" class="error">{error}</p>{/if}
+</Modal>
+
+<style>meter { width: 100%; margin: 1rem 0; }</style>
