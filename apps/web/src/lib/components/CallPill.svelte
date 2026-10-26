<script lang="ts">
  import type { CallController } from '../call/controller.svelte';
  import Avatar from './Avatar.svelte';
  import Icon from './Icon.svelte';
  import Meter from './Meter.svelte';
  let {
    call,
    self,
    expand,
    notice,
  }: {
    call: CallController;
    self?: { name: string; color: string; avatarUrl?: string } | null;
    expand: () => void;
    notice: (text: string) => void;
  } = $props();
  let x = $state<number | null>(null),
    y = $state<number | null>(null),
    dragging = $state(false);
  const seconds = $derived(Math.max(0, Math.floor((call.clock - call.startedAt) / 1000)));
  const time = $derived(
    `${String(Math.floor(seconds / 60)).padStart(2, '0')}:${String(seconds % 60).padStart(2, '0')}`,
  );
  let origin = { x: 0, y: 0, startX: 0, startY: 0 };
  function moveTo(left: number, top: number) {
    x = Math.max(12, Math.min(window.innerWidth - 320, left));
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

{#if call.error}<div class="toast" role="alert">
    {call.error}<button class="button small" onclick={() => call.resumeAudio()}>Resume audio</button>
  </div>{/if}

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
        }}><Icon name="grip" size={14} /></button
      ><Avatar
        url={self?.avatarUrl}
        name={self?.name ?? 'You'}
        color={self?.color}
        size={28}
        speaking={!call.muted && (call.levels[call.userId] ?? 0) > 0.3}
      /><button class="pill-room" onclick={expand}
        ><strong>{call.roomName}</strong><span
          ><i class="live-dot"></i><span class="readout">{time}</span><span class="pill-quality"
            >{call.quality}</span
          ></span
        ></button
      ><Meter level={call.muted ? 0 : (call.levels[call.userId] ?? 0)} segments={5} /><button
        class="icon-button"
        aria-label="Expand call"
        title="Expand"
        onclick={expand}><Icon name="expand" size={15} /></button
      >
    </div>
    <div class="pill-controls">
      <button
        class:control-off={call.muted}
        aria-label={call.muted ? 'Unmute microphone' : 'Mute microphone'}
        title={call.muted ? 'Unmute' : 'Mute'}
        aria-pressed={call.muted}
        onclick={() => call.toggleMute()}><Icon name={call.muted ? 'micOff' : 'mic'} size={16} /></button
      ><button
        class:control-off={call.deafened}
        aria-label={call.deafened ? 'Undeafen' : 'Deafen'}
        title={call.deafened ? 'Undeafen' : 'Deafen'}
        aria-pressed={call.deafened}
        onclick={() => call.toggleDeafen()}
        ><Icon name={call.deafened ? 'deafen' : 'headphones'} size={16} /></button
      ><button
        class:control-on={call.sharing}
        aria-label={call.sharing ? 'Stop screen sharing' : 'Share screen'}
        title={call.sharing ? 'Stop sharing' : 'Share screen'}
        aria-pressed={call.sharing}
        onclick={() => {
          call.toggleShare();
          if (call.sharing) expand();
        }}><Icon name="share" size={16} /></button
      ><button
        aria-label="Pop out call"
        title="Pop out"
        onclick={() => notice('Pop-out windows are coming to the desktop app.')}
        ><Icon name="external" size={15} /></button
      ><button class="leave-control" aria-label="Leave call" title="Leave" onclick={() => call.leave()}
        ><Icon name="leave" size={16} /></button
      >
    </div>
  </div>{/if}
