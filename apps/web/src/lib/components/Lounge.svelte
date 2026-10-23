<script lang="ts">
  import { api, useQuery, type RoomSummary } from '../data';
  import type { CallController } from '../call/controller.svelte';
  import { spatialAudio, type Point } from '../logic/lounge';
  import Avatar from './Avatar.svelte';
  import Icon from './Icon.svelte';
  let { room, call, join }: { room: RoomSummary; call: CallController; join: (room: RoomSummary) => void } = $props();
  const people = useQuery(api.voice.participants, () => ({ room: room._id }));
  let me = $state<Point>({ x: 400, y: 330 }), dragging = $state(false), canvas: HTMLDivElement;
  const positions: Record<string, Point> = { leo: { x: 260, y: 210 }, noah: { x: 590, y: 190 }, jules: { x: 640, y: 310 }, maya: { x: 160, y: 320 }, sofia: { x: 240, y: 350 } };
  const others = $derived((people.data ?? []).filter(p => p.user !== 'you').map((p, i) => ({ ...p, pos: positions[p.user] ?? { x: 180 + i * 140, y: 200 }, mix: spatialAudio(me, positions[p.user] ?? { x: 180 + i * 140, y: 200 }) })));
  $effect(() => { if (call.room === room._id) for (const p of others) call.setSpatialAudio(p.user, p.mix); });
  function move(e: PointerEvent) { if (!dragging) return; const r = canvas.getBoundingClientRect(); me = { x: Math.max(40, Math.min(760, (e.clientX - r.left) / r.width * 800)), y: Math.max(40, Math.min(460, (e.clientY - r.top) / r.height * 500)) }; }
</script>
<div class="lounge-view"><div class="lounge-intro"><div class="eyebrow"><Icon name="sprout" size={15}/> GROW A CONVERSATION</div><h1>The greenhouse.</h1><p>A few familiar faces. A little space to wander.</p><span class="lounge-instruction"><Icon name="compass" size={16}/>Drag yourself closer to join a conversation.</span></div>
  <div class="lounge-canvas" bind:this={canvas} role="group" aria-label="Spatial lounge canvas" onpointermove={move} onpointerup={() => dragging = false} onpointercancel={() => dragging = false}>
    <span class="canvas-label">A PLACE TO JUST BE</span><span class="canvas-plant plant-one">✳︎</span><span class="canvas-plant plant-two">✳︎</span><span class="lounge-table table-one"></span><span class="lounge-table table-two"></span><span class="canvas-coffee">☕</span>
    <div class="hearing-range" style:left={`${me.x / 8}%`} style:top={`${me.y / 5}%`}><span>Your hearing range</span></div>
    {#each others as p}<div class="lounge-person" style:left={`${p.pos.x / 8}%`} style:top={`${p.pos.y / 5}%`}><Avatar name={p.name} color={p.color} size={54} speaking={(call.levels[p.user] ?? 0) > .4}/><strong>{p.name.split(' ')[0]}</strong><small>{Math.round(p.mix.volume * 100)}% volume · {p.mix.pan < -.1 ? 'left' : p.mix.pan > .1 ? 'right' : 'center'}</small></div>{/each}
    <button class="lounge-self" class:dragging aria-label="Your position; drag or use arrow keys to move" style:left={`${me.x / 8}%`} style:top={`${me.y / 5}%`} onpointerdown={e => { dragging = true; e.currentTarget.setPointerCapture(e.pointerId); }} onpointermove={move} onpointerup={() => dragging = false} onkeydown={e => { if (e.key.startsWith('Arrow')) { e.preventDefault(); me = { x: Math.max(40, Math.min(760, me.x + (e.key === 'ArrowLeft' ? -20 : e.key === 'ArrowRight' ? 20 : 0))), y: Math.max(40, Math.min(460, me.y + (e.key === 'ArrowUp' ? -20 : e.key === 'ArrowDown' ? 20 : 0))) }; } }}><Avatar name="Alex Morgan" color="#b9a2e7" size={58} speaking={!call.muted && (call.levels.you ?? 0) > .5}/><strong>You <Icon name="grip" size={12}/></strong></button>
    <div class="canvas-footer"><span class="live-dot"></span>{others.length} people growing good ideas<span>Spatial audio · Headphones feel best</span></div>
  </div>
  <div class="lounge-bottom"><div><Icon name="headphones" size={24}/><span><strong>Like being in the same room.</strong><small>Closer voices get louder. Wander away for a quieter moment.</small></span></div>{#if call.room !== room._id}<button class="button primary" onclick={() => join(room)}><Icon name="headphones" size={17}/>Come on in<Icon name="upRight" size={15}/></button>{:else}<span class="lounge-connected"><span class="live-dot"></span>You’re right here.</span>{/if}</div>
</div>
