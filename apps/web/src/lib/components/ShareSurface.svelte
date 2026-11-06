<script lang="ts">
  import { onDestroy } from 'svelte';
  import type { CallController } from '../call/controller.svelte';
  import CallVideo from './CallVideo.svelte';

  let {
    call,
    stream,
    mid,
    native = false,
    drawing,
    zoom,
    participantName,
  }: {
    call: CallController;
    stream: MediaStream | null;
    mid: string;
    native?: boolean;
    drawing: boolean;
    zoom: number;
    participantName: (user: string) => string;
  } = $props();
  let viewport = $state({ width: 1, height: 1 });
  let source = $state({ width: 1920, height: 1080 });
  let frame: HTMLDivElement;
  let previous: [number, number] | undefined;
  let pointer: [number, number] | undefined;
  let pointerTimer: ReturnType<typeof setTimeout> | undefined;
  let pendingStroke: [number, number][] = [];
  let strokeTimer: ReturnType<typeof setTimeout> | undefined;
  const scale = $derived(Math.min(viewport.width / source.width, viewport.height / source.height) * zoom);

  function measure(node: HTMLDivElement) {
    const observer = new ResizeObserver(([entry]) => {
      viewport = { width: entry.contentRect.width, height: entry.contentRect.height };
    });
    observer.observe(node);
    return { destroy: () => observer.disconnect() };
  }
  function dimensions(width: number, height: number) {
    if (width > 0 && height > 0 && (width !== source.width || height !== source.height))
      source = { width, height };
  }
  function imageLoaded(event: Event) {
    const image = event.currentTarget as HTMLImageElement;
    dimensions(image.naturalWidth, image.naturalHeight);
  }
  function position(event: PointerEvent): [number, number] {
    const rect = frame.getBoundingClientRect();
    return [
      Math.max(0, Math.min(1, (event.clientX - rect.left) / rect.width)),
      Math.max(0, Math.min(1, (event.clientY - rect.top) / rect.height)),
    ];
  }
  function point(pos: [number, number] | null) {
    call.send({ type: 'pointer', participantId: call.userId, shareMid: mid, pos });
  }
  function hidePointer() {
    clearTimeout(pointerTimer);
    pointerTimer = undefined;
    pointer = undefined;
    point(null);
  }
  function draw(points: [number, number][]) {
    call.send({ type: 'stroke', participantId: call.userId, shareMid: mid, points, color: '#ffcc33' });
  }
  function flushStroke() {
    clearTimeout(strokeTimer);
    strokeTimer = undefined;
    if (pendingStroke.length > 1) draw(pendingStroke);
    pendingStroke = [];
  }
  function move(event: PointerEvent) {
    const pos = position(event);
    if (previous) {
      if (!pendingStroke.length) pendingStroke.push(previous);
      pendingStroke.push(pos);
      previous = pos;
      if (pendingStroke.length >= 256) flushStroke();
      else if (!strokeTimer) strokeTimer = setTimeout(flushStroke, 50);
    } else {
      // Leave room in the SFU's event budget for drawing, reactions, and hand raises.
      pointer = pos;
      if (!pointerTimer)
        pointerTimer = setTimeout(() => {
          pointerTimer = undefined;
          if (pointer) point(pointer);
        }, 50);
    }
  }
  onDestroy(() => {
    clearTimeout(pointerTimer);
    clearTimeout(strokeTimer);
  });
</script>

<!-- svelte-ignore a11y_no_noninteractive_tabindex, a11y_no_noninteractive_element_interactions (Interactive annotation surface supports pointer and keyboard input.) -->
<div
  class="shared-screen"
  data-share-mid={mid}
  use:measure
  role="application"
  aria-label="Shared screen; move to point, enable drawing to annotate"
  tabindex="0"
  onkeydown={(event) => {
    if (event.key === 'Enter') point([0.5, 0.5]);
    if (event.key === 'd')
      draw([
        [0.4, 0.6],
        [0.5, 0.5],
        [0.6, 0.6],
      ]);
  }}
>
  <div
    class="share-scroll"
    style:min-width={`${source.width * scale}px`}
    style:min-height={`${source.height * scale}px`}
  >
    <div
      class="share-frame"
      class:drawing
      bind:this={frame}
      style:width={`${source.width * scale}px`}
      style:height={`${source.height * scale}px`}
      onpointermove={move}
      onpointerdown={(event) => {
        if (drawing && event.button === 0) {
          event.preventDefault();
          hidePointer();
          previous = position(event);
          frame.setPointerCapture(event.pointerId);
        }
      }}
      onpointerup={(event) => {
        if (previous) {
          if (!pendingStroke.length) pendingStroke.push(previous);
          pendingStroke.push(position(event));
          flushStroke();
        }
        previous = undefined;
        if (frame.hasPointerCapture(event.pointerId)) frame.releasePointerCapture(event.pointerId);
      }}
      onpointercancel={() => {
        flushStroke();
        previous = undefined;
        hidePointer();
      }}
      onpointerleave={() => {
        if (!previous) hidePointer();
      }}
      role="presentation"
    >
      {#if stream || native}
        <CallVideo {stream} {mid} {call} onDimensions={dimensions} minHeight={2160} />
      {:else}
        <img src="/direction.svg" alt="Shared screen preview" draggable="false" onload={imageLoaded} />
      {/if}
      <svg class="annotations" viewBox="0 0 1000 1000" preserveAspectRatio="none" aria-hidden="true">
        {#each call.strokes.filter((stroke) => stroke.shareMid === mid) as stroke (stroke.id)}
          <polyline
            points={stroke.points.map((p) => `${p[0] * 1000},${p[1] * 1000}`).join(' ')}
            fill="none"
            stroke={stroke.color}
            stroke-width="4"
            stroke-linecap="round"
            class="annotation-stroke"
          />
        {/each}
      </svg>
      {#each Object.entries(call.pointers).filter(([, p]) => p.shareMid === mid) as [user, pointer]}
        <span
          class="share-pointer"
          style:left={`${pointer.pos.x * 100}%`}
          style:top={`${pointer.pos.y * 100}%`}
        >
          <span></span><strong>{user === call.userId ? 'You' : participantName(user)}</strong>
        </span>
      {/each}
    </div>
  </div>
</div>
