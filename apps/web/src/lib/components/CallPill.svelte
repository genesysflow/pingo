<script lang="ts">
  import type { CallController } from '../call/controller.svelte';
  import Avatar from './Avatar.svelte';
  import Icon from './Icon.svelte';
  let { call, expand, notice }: { call: CallController; expand: () => void; notice: (text: string) => void } =
    $props();
  let x = $state<number | null>(null),
    y = $state<number | null>(null),
    dragging = $state(false);
  const seconds = $derived(Math.max(0, Math.floor((call.clock - call.startedAt) / 1000)));
  const time = $derived(`${Math.floor(seconds / 60)}:${String(seconds % 60).padStart(2, '0')}`);
  let origin = { x: 0, y: 0, startX: 0, startY: 0 };
  function moveTo(left: number, top: number) {
    x = Math.max(12, Math.min(window.innerWidth - 340, left));
    y = Math.max(12, Math.min(window.innerHeight - 90, top));
  }
  function down(e: PointerEvent) {
    const rect =
      e.currentTarget instanceof HTMLElement
        ? e.currentTarget.closest('.call-pill')!.getBoundingClientRect()
        : null;
    if (!rect) return;
    dragging = true;
    origin = { x: e.clientX, y: e.clientY, startX: rect.left, startY: rect.top };
    (e.currentTarget as HTMLElement).setPointerCapture(e.pointerId);
  }
</script>

<svelte:window
  onresize={() => {
    if (x !== null && y !== null) moveTo(x, y);
  }}
/>
{#if call.room}<div
    class="call-pill"
    class:dragging
    style:left={x === null ? undefined : `${x}px`}
    style:top={y === null ? undefined : `${y}px`}
    style:bottom={y === null ? '22px' : 'auto'}
    style:right={x === null ? '28px' : 'auto'}
    role="region"
    aria-label="Call controls"
  >
    <div class="pill-top">
      <button
        class="pill-grip"
        aria-label="Move call controls; use arrow keys"
        onpointerdown={down}
        onpointermove={(e) => {
          if (dragging) moveTo(origin.startX + e.clientX - origin.x, origin.startY + e.clientY - origin.y);
        }}
        onpointerup={() => (dragging = false)}
        onpointercancel={() => (dragging = false)}
        onkeydown={(e) => {
          if (e.key.startsWith('Arrow')) {
            e.preventDefault();
            const r = e.currentTarget.closest('.call-pill')!.getBoundingClientRect();
            moveTo(
              r.left + (e.key === 'ArrowLeft' ? -20 : e.key === 'ArrowRight' ? 20 : 0),
              r.top + (e.key === 'ArrowUp' ? -20 : e.key === 'ArrowDown' ? 20 : 0),
            );
          }
        }}><Icon name="grip" size={16} /></button
      ><Avatar
        name="Alex Morgan"
        color="#b9a2e7"
        size={31}
        speaking={!call.muted && (call.levels.you ?? 0) > 0.3}
      /><button class="pill-room" onclick={expand}
        ><strong>{call.roomName}</strong><span
          ><i class="live-dot"></i>{time}<span class="pill-quality">Your audio: {call.quality}</span></span
        ></button
      ><button class="icon-button" aria-label="Expand call" onclick={expand}
        ><Icon name="expand" size={16} /></button
      >
    </div>
    <div class="pill-controls">
      <button
        class:control-off={call.muted}
        aria-label={call.muted ? 'Unmute microphone' : 'Mute microphone'}
        aria-pressed={call.muted}
        onclick={() => call.toggleMute()}><Icon name={call.muted ? 'micOff' : 'mic'} size={17} /></button
      ><button
        class:control-off={call.deafened}
        aria-label={call.deafened ? 'Undeafen' : 'Deafen'}
        aria-pressed={call.deafened}
        onclick={() => call.toggleDeafen()}
        ><Icon name={call.deafened ? 'deafen' : 'headphones'} size={17} /></button
      ><button
        class:control-on={call.sharing}
        aria-label={call.sharing ? 'Stop screen sharing' : 'Share screen'}
        aria-pressed={call.sharing}
        onclick={() => {
          call.toggleShare();
          if (call.sharing) expand();
        }}><Icon name="share" size={17} /></button
      ><button
        aria-label="Pop out call"
        onclick={() => notice('Pop-out is coming with the desktop app. Your call stays here for now.')}
        ><Icon name="external" size={16} /></button
      ><button class="leave-control" aria-label="Leave call" onclick={() => call.leave()}
        ><Icon name="leave" size={18} /></button
      >
    </div>
  </div>{/if}
