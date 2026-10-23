<script lang="ts">
  import { onMount } from 'svelte';
  import { api, useQuery, isDemo, type RoomSummary } from '../data';
  import type { CallController } from '../call/controller.svelte';
  import CallVideo from './CallVideo.svelte';
  import Avatar from './Avatar.svelte';
  import Icon from './Icon.svelte';
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
  const hasShare = $derived(!!screenStream || call.sharing);
  const seconds = $derived(Math.max(0, Math.floor((call.clock - call.startedAt) / 1000)));
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
          color: '#a889ed',
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
  {#if call.connection !== 'connected'}<p role="status">
      {call.connection === 'reconnecting'
        ? 'Reconnecting to your people…'
        : call.connection === 'failed'
          ? 'Call disconnected'
          : 'Connecting…'}
    </p>{/if}
  {#if call.error}<p class="error" role="alert">{call.error}</p>
    <button class="button small" onclick={() => call.resumeAudio()}>Resume call audio</button>{/if}
  <div class="stage-intro">
    <div>
      <span class="eyebrow"><span class="live-dot"></span> A LITTLE CLOSER</span>
      <h1>Good company.<span>No agenda needed.</span></h1>
    </div>
    <button class="button" onclick={collapse}><Icon name="collapse" size={15} />Keep browsing</button>
  </div>
  <div class="stage-toolbar">
    <span
      ><span class="live-dot"></span>Live in {room.name}<span class="dot-separator">·</span>{Math.floor(
        seconds / 60,
      )}:{String(seconds % 60).padStart(2, '0')}<span class="dot-separator">·</span>{participants.data
        ?.length ?? 0} people</span
    ><button
      class="text-button"
      onclick={() => notice('Pop-out is coming with the desktop app. Your call stays here for now.')}
      ><Icon name="external" size={15} />Pop out</button
    >
  </div>
  {#if hasShare}<div class="screen-share-header">
      <span
        ><Icon name="share" size={15} />{call.sharing ? 'You’re sharing your screen' : 'Shared screen'}<span
          class="muted">· Point to show, draw to explain</span
        ></span
      >
      <div class="segmented">
        {#each ['Text', 'Motion'] as mode}<button
            class:active={call.shareMode === mode}
            onclick={() => call.setShareMode(mode as 'Text' | 'Motion')}>{mode}</button
          >{/each}
      </div>
      <button
        class:active={drawing}
        class="icon-button"
        aria-label="Draw on screen"
        aria-pressed={drawing}
        onclick={() => (drawing = !drawing)}><Icon name="edit" size={16} /></button
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
            color: '#a889ed',
          });
      }}
    >
      {#if screenStream}{#key screenStream}<CallVideo
            stream={screenStream}
            mid={screen?.mid}
            {call}
          />{/key}{:else}<div class="screen-browser">
          <i></i><i></i><i></i><span>fieldwork.design / a-new-chapter</span><Icon name="lock" size={10} />
        </div>
        <img
          src="/direction.svg"
          alt="Fieldwork design preview: A little room to grow"
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
              : (participants.data?.find((p) => p.user === user)?.name.split(' ')[0] ?? user)}</strong
          ></span
        >{/each}
    </div>{/if}
  <div class="stage-tiles" class:with-share={hasShare} class:has-spotlight={!!spotlight}>
    {#each participants.data ?? [] as p (p.user)}
      {@const camera = call.media.find((m) => m.user === p.user && m.kind === 'camera')}
      <div
        class="stage-tile"
        class:spotlight={spotlight === p.user}
        class:speaking={(call.levels[p.user] ?? 0) > 0.4 && !p.muted}
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
          ><Avatar
            name={p.name}
            color={p.color}
            size={hasShare ? 50 : spotlight === p.user ? 100 : 78}
            speaking={(call.levels[p.user] ?? 0) > 0.4 && !p.muted}
          /></button
        >
        <div class="tile-label">
          <strong>{p.name}{p.user === call.userId ? ' (you)' : ''}</strong
          >{#if call.raised.includes(p.user)}<span>✋</span>{/if}{#if p.muted}<Icon
              name="micOff"
              size={14}
            />{:else}<span class="audio-bars"
              >{#each [0, 1, 2, 3] as i}<i
                  style:height={`${4 + (call.levels[p.user] ?? 0) * (i % 2 ? 16 : 10)}px`}
                ></i>{/each}</span
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
  <div class="call-reactions" aria-live="polite">
    {#each call.reactions as r (r.id)}<span
        class="floating-reaction"
        style:left={`${20 + ((r.id * 19) % 60)}%`}
        >{r.emoji}<small>{r.user === call.userId ? 'You' : (participants.data?.find(p => p.user === r.user)?.name ?? 'Someone')}</small></span
      >{/each}
  </div>
  <div class="stage-bottom">
    <div class="quality-label">
      <Icon name="audio" size={18} /><span
        >Your audio: <strong>{call.quality}</strong><small
          >{call.audio.studio ? 'Studio · Every little detail' : 'Clear voices. A little less noise.'}</small
        ></span
      >
    </div>
    <div class="stage-controls">
      <button
        class:control-off={call.muted}
        aria-label={call.muted ? 'Unmute' : 'Mute'}
        onclick={() => call.toggleMute()}><Icon name={call.muted ? 'micOff' : 'mic'} /></button
      ><button
        class:control-off={call.deafened}
        aria-label="Toggle deafen"
        onclick={() => call.toggleDeafen()}><Icon name="headphones" /></button
      ><button
        class:control-on={call.sharing}
        aria-label={call.sharing ? 'Stop sharing' : 'Share your screen'}
        onclick={() => call.toggleShare()}><Icon name="share" /></button
      ><button
        class:control-on={call.raised.includes(call.userId)}
        aria-label="Raise hand"
        aria-pressed={call.raised.includes(call.userId)}
        onclick={() =>
          call.send({
            type: 'raiseHand',
            participantId: call.userId,
            raised: !call.raised.includes(call.userId),
          })}><Icon name="hand" /></button
      ><button
        class="leave-control"
        aria-label="Leave this call"
        onclick={() => {
          call.leave();
          collapse();
        }}><Icon name="leave" /></button
      >
      <button
        class:control-on={call.camera}
        aria-label={call.camera ? 'Turn camera off' : 'Turn camera on'}
        onclick={() => call.toggleCamera()}><Icon name="video" /></button
      >
    </div>
    <div class="stage-emoji">
      {#each ['💜', '✨', '🙌', '😂'] as em}<button
          aria-label={`Send ${em} in call`}
          onclick={() => call.send({ type: 'reaction', participantId: call.userId, emoji: em })}>{em}</button
        >{/each}
    </div>
  </div>
</div>
