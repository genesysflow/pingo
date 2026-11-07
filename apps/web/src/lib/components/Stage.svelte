<script lang="ts">
  import { api, useQuery, type RoomSummary } from '../data';
  import type { CallController } from '../call/controller.svelte';
  import CallVideo from './CallVideo.svelte';
  import CameraControl from './CameraControl.svelte';
  import ShareSurface from './ShareSurface.svelte';
  import Avatar from './Avatar.svelte';
  import Icon from './Icon.svelte';
  import Meter from './Meter.svelte';
  let { call, room, collapse }: { call: CallController; room: RoomSummary; collapse: () => void } = $props();
  const participants = useQuery(api.voice.participants, () => ({ room: room._id }));
  let spotlight = $state(''),
    drawing = $state(false),
    zoom = $state(1),
    expanded = $state(false);
  const screen = $derived(call.media.find((m) => m.kind === 'screen'));
  const screenStream = $derived(screen?.stream ?? call.localScreen);
  const screenMid = $derived(screen?.mid ?? call.screenMid);
  const hasShare = $derived(!!screen || !!screenStream || call.sharing);
  $effect(() => {
    if (!hasShare) {
      expanded = false;
      zoom = 1;
      drawing = false;
    }
  });
  const seconds = $derived(Math.max(0, Math.floor((call.clock - call.startedAt) / 1000)));
  const clock = $derived(
    `${String(Math.floor(seconds / 60)).padStart(2, '0')}:${String(seconds % 60).padStart(2, '0')}`,
  );
  const people = $derived(participants.data ?? []);
  // A grid that keeps tiles at 16:10 and as large as the stage allows.
  const columns = $derived(
    hasShare
      ? Math.min(Math.max(people.length, 1), 6)
      : people.length <= 1
        ? 1
        : people.length <= 4
          ? 2
          : people.length <= 9
            ? 3
            : 4,
  );
  const rows = $derived(hasShare ? 1 : Math.max(1, Math.ceil(people.length / columns)));
  const suppressionOrder = ['Off', 'Gentle', 'Strong'] as const;
  function cycleSuppression() {
    const next = suppressionOrder[(suppressionOrder.indexOf(call.audio.suppression) + 1) % 3];
    call.configureAudio({ ...call.audio, suppression: next });
  }
</script>

<svelte:window
  onkeydowncapture={(event) => {
    if (expanded && event.key === 'Escape') {
      event.stopPropagation();
      expanded = false;
    }
  }}
/>

<div class="stage-view" class:share-expanded={hasShare && expanded}>
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
      <button class="icon-button" aria-label="Minimize call" title="Minimize" onclick={collapse}
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
      <div class="segmented share-tools" role="group" aria-label="Screen annotations">
        <button
          class:active={!drawing}
          aria-pressed={!drawing}
          aria-label="Point on screen"
          onclick={() => (drawing = false)}><Icon name="pointer" size={15} /><span>Pointer</span></button
        >
        <button
          class:active={drawing}
          aria-pressed={drawing}
          aria-label="Draw on screen"
          onclick={() => (drawing = true)}><Icon name="edit" size={15} /><span>Draw</span></button
        >
      </div>
      <div class="share-zoom" role="group" aria-label="Screen zoom">
        <button
          class="icon-button"
          aria-label="Zoom out"
          disabled={zoom <= 1}
          onclick={() => (zoom = Math.max(1, zoom - 0.25))}><Icon name="minus" size={15} /></button
        >
        <button
          class="zoom-fit"
          aria-label="Fit shared screen"
          title="Fit shared screen"
          onclick={() => (zoom = 1)}>{zoom === 1 ? 'Fit' : `${Math.round(zoom * 100)}%`}</button
        >
        <button
          class="icon-button"
          aria-label="Zoom in"
          disabled={zoom >= 3}
          onclick={() => (zoom = Math.min(3, zoom + 0.25))}><Icon name="plus" size={15} /></button
        >
      </div>
      <button
        class="icon-button"
        aria-label={expanded ? 'Restore shared screen' : 'Expand shared screen'}
        title={expanded ? 'Restore shared screen (Esc)' : 'Expand shared screen'}
        aria-pressed={expanded}
        onclick={() => (expanded = !expanded)}
        ><Icon name={expanded ? 'collapse' : 'expand'} size={16} /></button
      >
    </div>
    {#key screenMid}<ShareSurface
        {call}
        stream={screenStream}
        mid={screenMid}
        native={screen?.native}
        {drawing}
        {zoom}
        participantName={(user) => people.find((p) => p.user === user)?.name.split(' ')[0] ?? user}
      />{/key}
  {/if}
  <div class="stage-area" class:with-share={hasShare}>
    <div
      class="stage-tiles"
      class:with-share={hasShare}
      style:--cols={columns}
      style:--rows={rows}
      style:--max-width={people.length <= 1 && !hasShare ? '880px' : '100%'}
    >
      {#each people as p (p.user)}
        {@const camera =
          p.user === call.userId && call.localCamera
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
            >{#if !camera}<Avatar name={p.name} color={p.color} size={84} />{/if}</button
          >
          <div class="tile-label">
            {#if p.muted}<Icon name="micOff" size={13} />{:else}<Meter {level} segments={5} />{/if}<strong
              >{p.name}</strong
            >{#if p.user === call.userId}<span class="you-tag">you</span
              >{/if}{#if call.raised.includes(p.user)}<span class="hand">✋</span>{/if}
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
    {#each call.reactions as r (r.id)}<span
        class="floating-reaction"
        style:left={`${20 + ((r.id * 19) % 60)}%`}
        >{r.emoji}<small
          >{r.user === call.userId
            ? 'You'
            : (people.find((p) => p.user === r.user)?.name ?? 'Someone')}</small
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
          onclick={cycleSuppression}>Noise {call.audio.suppression}</button
        >{/if}
      <Meter
        level={call.muted ? 0 : (call.levels[call.userId] ?? 0)}
        segments={8}
        label="Your microphone level"
      />
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
      ><CameraControl {call} /><button
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
