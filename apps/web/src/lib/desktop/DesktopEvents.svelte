<script lang="ts">
  import { onMount } from 'svelte';
  import { invoke } from '@tauri-apps/api/core';
  import { api, useQuery, useAuth } from '../data';
  import { watchInvites } from './bridge';
  import type { CallController } from '../call/controller.svelte';
  import { TauriCallController } from '../call/tauri.svelte';
  let { call }: { call: CallController } = $props();
  function key(e: KeyboardEvent, pressed: boolean) {
    if (e.code !== 'Space' || e.repeat || !call.room || !call.audio.pushToTalk) return;
    if (pressed && e.target instanceof HTMLElement && (e.target.isContentEditable || ['INPUT', 'TEXTAREA', 'SELECT', 'BUTTON'].includes(e.target.tagName))) return;
    e.preventDefault();
    if (call instanceof TauriCallController) call.pushToTalk(pressed);
  }
  const auth = useAuth();
  const inbox = useQuery(api.inbox.list, () => auth.isAuthenticated ? {} : 'skip');
  let seen: Set<string> | undefined;
  $effect(() => {
    if (!auth.isAuthenticated) { seen = undefined; return; }
    const items = inbox.data;
    if (!items) return;
    const previous = seen;
    seen = new Set(items.map(i => i._id));
    if (!previous || localStorage.getItem('pingo-notify-desktop') === 'false') return;
    for (const item of items) {
      if (previous.has(item._id) || !['mention', 'dm'].includes(item.kind)) continue;
      if (document.hasFocus() && document.visibilityState === 'visible') continue;
      void invoke('desktop_notify', {
        title: item.kind === 'dm' ? `${item.message.author.name} sent a message` : `${item.message.author.name} mentioned you`,
        body: item.message.body.slice(0, 4096),
      }).catch(console.error);
    }
  });
  onMount(() => {
    let disposed = false, stop: (() => void) | undefined;
    void watchInvites().then(unlisten => { if (disposed) unlisten(); else stop = unlisten; });
    return () => { disposed = true; stop?.(); };
  });
</script>

<svelte:window onkeydown={e => key(e, true)} onkeyup={e => key(e, false)} onblur={() => { if (call instanceof TauriCallController) call.pushToTalk(false); }} />
