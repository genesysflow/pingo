<script lang="ts">
  import { onMount } from 'svelte';
  import { api, useQuery, isDemo, type RoomSummary } from '../data';
  import type { CallController } from '../call/controller.svelte';
  import CallVideo from './CallVideo.svelte';
  import Avatar from './Avatar.svelte';
  import Icon from './Icon.svelte';
  import Meter from './Meter.svelte';
  let {
    call,
    room,
    collapse,
    notice,
  }: { call: CallController; room: RoomSummary; collapse: () => void; notice: (text: string) => void } =
    $props();
  const participants = useQuery(api.voice.participants, () => ({ room: room._id }));
  let spotlight = $state(''),
    drawing = $state(false),
    isDrawing = false,
    stroke: [number, number][] = [],
    share = $state<HTMLDivElement>()!;
  const screen = $derived(call.media.find((m) => m.kind === 'screen'));
  const screenStream = $derived(screen?.stream ?? call.localScreen);
  const screenMid = $derived(screen?.mid ?? call.screenMid);
  const hasShare = $derived(!!screen || !!screenStream || call.sharing);
  const seconds = $derived(Math.max(0, Math.floor((call.clock - call.startedAt) / 1000)));
  const clock = $derived(
    `${String(Math.floor(seconds / 60)).padStart(2, '0')}:${String(seconds % 60).padStart(2, '0')}`,
  );
  const people = $derived(participants.data ?? []);
  // A grid that keeps tiles at 16:10 and as large as the stage allows.
  const columns = $derived(
    hasShare ? Math.min(Math.max(people.length, 1), 6) : people.length <= 1 ? 1 : people.length <= 4 ? 2 : people.length <= 9 ? 3 : 4,
  );
  const rows = $derived(hasShare ? 1 : Math.max(1, Math.ceil(people.length / columns)));
  const suppressionOrder = ['Off', 'Gentle', 'Strong'] as const;
  function cycleSuppression() {
    const next = suppressionOrder[(suppressionOrder.indexOf(call.audio.suppression) + 1) % 3];
    call.configureAudio({ ...call.audio, suppression: next });
  }
  function position(e: PointerEvent): [number, number] {
    const r = share.getBoundingClientRect();
    return [
      Math.max(0, Math.min(1, (e.clientX - r.left) / r.width)),
      Math.max(0, Math.min(1, (e.clientY - r.top) / r.height)),
    ];
  }
  function move(e: PointerEvent) {
    const pos = position(e);
    call.send({ type: 'pointer', participantId: call.userId, shareMid: screenMid, pos });
    if (isDrawing) {
      stroke.push(pos);
      if (stroke.length > 1)
        call.send({
          type: 'stroke',
          participantId: call.userId,
          shareMid: screenMid,
          points: stroke.slice(-2),
          color: 'var(--accent)',
        });
    }
  }
  onMount(() => {
    if (!isDemo) return;
    const timer = setInterval(() => {
      if (call.sharing) {
        call.send({
          type: 'pointer',
          participantId: 'maya',
          shareMid: screenMid,
          pos: [0.58 + Math.sin(Date.now() / 3000) * 0.1, 0.6 + Math.cos(Date.now() / 4000) * 0.08],
        });
      }
    }, 500);
    return () => clearInterval(timer);
  });
</script>

<div class="stage-view">
  <div class="stage-bar">
    <span class="live-chip"><span class="live-dot"></span>Live</span>
    <span class="readout">{clock}</span>
    <span class="readout muted">{people.length} in call</span>
    {#if call.connection !== 'connected'}<span class="stage-status" role="status"
        >{call.connection === 'reconnecting'
          ? 'Reconnecting…'
          : call.connection === 'failed'
            ? 'Call disconnected'
            : 'Connecting…'}</span
      >{/if}
    <span class="stage-bar-actions">
      <button
        class="icon-button"
        aria-label="Pop out"
        title="Pop out"
        onclick={() => notice('Pop-out windows are coming to the desktop app.')}
        ><Icon name="external" size={16} /></button
      ><button class="icon-button" aria-label="Minimize call" title="Minimize" onclick={collapse}
        ><Icon name="collapse" size={16} /></button
      >
    </span>
  </div>
  {#if call.error}<div class="stage-error" role="alert">
      <span>{call.error}</span><button class="button small" onclick={() => call.resumeAudio()}
        >Resume call audio</button
      >
    </div>{/if}
  {#if hasShare}<div class="screen-share-header">
      <span
        ><Icon name="share" size={14} />{call.sharing ? 'You’re sharing your screen' : 'Shared screen'}<span
          class="muted">Move to point · draw to annotate</span
        ></span
      >
      <div class="segmented" role="group" aria-label="Share mode">
        {#each ['Text', 'Motion'] as mode}<button
            class:active={call.shareMode === mode}
            title={mode === 'Text' ? 'Sharp text, up to 4K at 15 fps' : 'Smooth motion, 1080p at 60 fps'}
            onclick={() => call.setShareMode(mode as 'Text' | 'Motion')}>{mode}</button
          >{/each}
      </div>
      <button
        class:active={drawing}
        class="icon-button"
        aria-label="Draw on screen"
        title="Draw"
        aria-pressed={drawing}
        onclick={() => (drawing = !drawing)}><Icon name="edit" size={15} /></button
      >
    </div>
    <!-- svelte-ignore a11y_no_noninteractive_tabindex, a11y_no_noninteractive_element_interactions (Interactive drawing surface supports pointer and keyboard input.) -->
    <div
      class="shared-screen"
      bind:this={share}
      class:drawing
      role="application"
      aria-label="Shared screen; move to point, enable drawing to annotate"
      tabindex="0"
      onpointermove={move}
      onpointerdown={(e) => {
        if (drawing) {
          isDrawing = true;
          stroke = [position(e)];
          share.setPointerCapture(e.pointerId);
        }
      }}
      onpointerup={() => (isDrawing = false)}
      onpointercancel={() => (isDrawing = false)}
      onpointerleave={() => {
        if (!isDrawing)
          call.send({ type: 'pointer', participantId: call.userId, shareMid: screenMid, pos: null });
      }}
      onkeydown={(e) => {
        if (e.key === 'Enter') {
          call.send({ type: 'pointer', participantId: call.userId, shareMid: screenMid, pos: [0.5, 0.5] });
        }
        if (e.key === 'd')
          call.send({
            type: 'stroke',
            participantId: call.userId,
            shareMid: screenMid,
            points: [
              [0.4, 0.6],
              [0.5, 0.5],
              [0.6, 0.6],
            ],
            color: 'var(--accent)',
          });
      }}
    >
      {#if screenStream || screen?.native}{#key screen?.mid ?? screenStream}<CallVideo
            stream={screenStream}
            mid={screen?.mid}
            {call}
          />{/key}{:else}<img
          src="/direction.svg"
          alt="Shared screen preview"
          draggable="false"
        />{/if}
      <svg class="annotations" viewBox="0 0 1000 1000" preserveAspectRatio="none" aria-hidden="true"
        >{#each call.strokes.filter((s) => s.shareMid === screenMid) as s (s.id)}<polyline
            points={s.points.map((p) => `${p[0] * 1000},${p[1] * 1000}`).join(' ')}
            fill="none"
            stroke={s.color}
            stroke-width="4"
            stroke-linecap="round"
            class="annotation-stroke"
          />{/each}</svg
      >
      {#each Object.entries(call.pointers).filter(([, p]) => p.shareMid === screenMid) as [user, pointer]}<span
          class="share-pointer"
          style:left={`${pointer.pos.x * 100}%`}
          style:top={`${pointer.pos.y * 100}%`}
          ><span></span><strong
            >{user === call.userId
              ? 'You'
              : (people.find((p) => p.user === user)?.name.split(' ')[0] ?? user)}</strong
          ></span
        >{/each}
    </div>{/if}
  <div class="stage-area" class:with-share={hasShare}>
    <div
      class="stage-tiles"
      class:with-share={hasShare}
      style:--cols={columns}
      style:--rows={rows}
      style:--max-width={people.length <= 1 && !hasShare ? '880px' : '100%'}
    >
      {#each people as p (p.user)}
        {@const camera = p.user === call.userId && call.localCamera
          ? { stream: call.localCamera, mid: '' }
          : call.media.find((m) => m.user === p.user && m.kind === 'camera')}
        {@const level = p.muted ? 0 : (call.levels[p.user] ?? 0)}
        <div
          class="stage-tile"
          class:spotlight={spotlight === p.user}
          class:speaking={level > 0.4}
          class:has-video={!!camera}
          style:--tile-color={p.color}
        >
          {#if camera}{#key camera.stream}<CallVideo
                stream={camera.stream}
                mid={camera.mid}
                {call}
                label={`${p.name}'s camera`}
              />{/key}{/if}
          <button
            class="tile-main"
            aria-label={`Spotlight ${p.name}`}
            onclick={() => (spotlight = spotlight === p.user ? '' : p.user)}
            >{#if !camera}<Avatar name={p.name} color={p.color} size={hasShare ? 40 : 84} />{/if}</button
          >
          <div class="tile-label">
            {#if p.muted}<Icon name="micOff" size={13} />{:else}<Meter {level} segments={5} />{/if}<strong
              >{p.name}</strong
            >{#if p.user === call.userId}<span class="you-tag">you</span>{/if}{#if call.raised.includes(p.user)}<span
                class="hand">✋</span
              >{/if}
          </div>
          <label class="tile-volume"
            ><span class="sr-only">Volume for {p.name}</span><Icon name="volume" size={12} /><input
              type="range"
              min="0"
              max="1.5"
              step=".05"
              value={call.volumes[p.user]?.volume ?? 1}
              oninput={(e) => call.setParticipantVolume(p.user, +e.currentTarget.value)}
            /></label
          >
        </div>{/each}
    </div>
    {#if people.length === 1 && !hasShare}<p class="stage-alone">
        You’re the only one here. People in this room can join from the sidebar.
      </p>{/if}
  </div>
  <div class="call-reactions" aria-live="polite">
    {#each call.reactions as r (r.id)}<span class="floating-reaction" style:left={`${20 + ((r.id * 19) % 60)}%`}
        >{r.emoji}<small
          >{r.user === call.userId ? 'You' : (people.find((p) => p.user === r.user)?.name ?? 'Someone')}</small
        ></span
      >{/each}
  </div>
  <div class="stage-dock">
    <div class="dock-status">
      <span class="quality {call.quality.toLowerCase()}" title="Connection quality"
        ><span class="live-dot"></span>{call.quality}</span
      >
      {#if call.audio.studio}<span class="dock-chip" title="Studio: stereo, no processing">Studio</span
        >{:else}<button
          class="dock-chip"
          class:off={call.audio.suppression === 'Off'}
          aria-label={`Noise suppression: ${call.audio.suppression}. Change`}
          title={`Noise suppression${call.noiseEngine === 'DeepFilterNet' ? ' (DeepFilterNet)' : call.noiseEngine === 'Browser' ? ' (browser filter)' : ''}`}
          onclick={cycleSuppression}>NS {call.audio.suppression}</button
        >{/if}
      <Meter level={call.muted ? 0 : (call.levels[call.userId] ?? 0)} segments={8} label="Your microphone level" />
    </div>
    <div class="stage-controls">
      <button
        class:control-off={call.muted}
        aria-label={call.muted ? 'Unmute' : 'Mute'}
        title={call.muted ? 'Unmute' : 'Mute'}
        onclick={() => call.toggleMute()}><Icon name={call.muted ? 'micOff' : 'mic'} size={18} /></button
      ><button
        class:control-off={call.deafened}
        aria-label="Toggle deafen"
        title={call.deafened ? 'Undeafen' : 'Deafen'}
        onclick={() => call.toggleDeafen()}
        ><Icon name={call.deafened ? 'deafen' : 'headphones'} size={18} /></button
      ><button
        class:control-on={call.camera}
        aria-label={call.camera ? 'Turn camera off' : 'Turn camera on'}
        title={call.camera ? 'Turn camera off' : 'Turn camera on'}
        onclick={() => call.toggleCamera()}><Icon name="video" size={18} /></button
      ><button
        class:control-on={call.sharing}
        aria-label={call.sharing ? 'Stop sharing' : 'Share your screen'}
        title={call.sharing ? 'Stop sharing' : 'Share your screen'}
        onclick={() => call.toggleShare()}><Icon name="share" size={18} /></button
      ><button
        class:control-on={call.raised.includes(call.userId)}
        aria-label="Raise hand"
        title="Raise hand"
        aria-pressed={call.raised.includes(call.userId)}
        onclick={() =>
          call.send({
            type: 'raiseHand',
            participantId: call.userId,
            raised: !call.raised.includes(call.userId),
          })}><Icon name="hand" size={18} /></button
      ><button
        class="leave-control"
        aria-label="Leave this call"
        title="Leave"
        onclick={() => {
          call.leave();
          collapse();
        }}><Icon name="leave" size={18} /><span>Leave</span></button
      >
    </div>
    <div class="stage-emoji">
      {#each ['👍', '😂', '🎉', '❤️'] as em}<button
          aria-label={`Send ${em} in call`}
          onclick={() => call.send({ type: 'reaction', participantId: call.userId, emoji: em })}>{em}</button
        >{/each}
    </div>
  </div>
</div>
