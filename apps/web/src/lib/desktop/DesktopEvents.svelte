<script lang="ts">
  import { onMount } from 'svelte';
  import { watchInvites } from './bridge';
  import type { CallController } from '../call/controller.svelte';
  import { TauriCallController } from '../call/tauri.svelte';
  let { call }: { call: CallController } = $props();
  function key(e: KeyboardEvent, pressed: boolean) {
    if (e.code !== 'Space' || e.repeat || !call.room || !call.audio.pushToTalk) return;
    if (
      pressed &&
      e.target instanceof HTMLElement &&
      (e.target.isContentEditable || ['INPUT', 'TEXTAREA', 'SELECT', 'BUTTON'].includes(e.target.tagName))
    )
      return;
    e.preventDefault();
    if (call instanceof TauriCallController) call.pushToTalk(pressed);
  }
  onMount(() => {
    let disposed = false,
      stop: (() => void) | undefined;
    void watchInvites().then((unlisten) => {
      if (disposed) unlisten();
      else stop = unlisten;
    });
    return () => {
      disposed = true;
      stop?.();
    };
  });
</script>

<svelte:window
  onkeydown={(e) => key(e, true)}
  onkeyup={(e) => key(e, false)}
  onblur={() => {
    if (call instanceof TauriCallController) call.pushToTalk(false);
  }}
/>
