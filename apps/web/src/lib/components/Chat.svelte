<script lang="ts">
  import { tick, untrack, onMount } from 'svelte';
  import {
    api,
    usePaginatedQuery,
    useQuery,
    useMutation,
    isDemo,
    errorMessage,
    type RoomSummary,
  } from '../data';
  import { isGrouped, sameDay } from '../logic/chat';
  import type { CallController } from '../call/controller.svelte';
  import Avatar from './Avatar.svelte';
  import Icon from './Icon.svelte';
  import Message from './Message.svelte';
  import Composer from './Composer.svelte';
  let {
    room,
    space,
    call,
    onThread,
    join,
    command,
  }: {
    room: RoomSummary;
    space: string;
    call: CallController;
    onThread: (id: string) => void;
    join: (room: RoomSummary) => void;
    command: (cmd: string) => void;
  } = $props();
  const query = usePaginatedQuery(api.messages.list, () => ({ room: room._id }), { initialNumItems: 35 });
  const typing = useQuery(api.typing.list, () => ({ room: room._id }));
  const markRead = useMutation(api.rooms.markRead);
  let scroller: HTMLDivElement;
  let scrollTop = $state(0),
    viewport = $state(600),
    measurements = $state<Record<string, number>>({}),
    atBottom = $state(true),
    newCount = $state(0);
  const messages = $derived([...query.results].reverse());
  const layout = $derived.by(() => {
    let offset = 0;
    return messages.map((m, i) => {
      const day = !i || !sameDay(messages[i - 1]._creationTime, m._creationTime);
      const grouped = !day && m._id !== unreadId && isGrouped(messages[i - 1], m);
      const size = measurements[m._id] ?? (m.attachments.length ? 390 : grouped ? 52 : 92);
      const row = { message: m, offset, size, grouped, day, i };
      offset += size;
      return row;
    });
  });
  const total = $derived(layout.length ? layout.at(-1)!.offset + layout.at(-1)!.size : 0);
  const visible = $derived(
    layout.filter(
      (row) => row.offset + row.size > scrollTop - 450 && row.offset < scrollTop + viewport + 450,
    ),
  );
  let previousRoom = '',
    previousNewest = '',
    previousOldest = '';
  let unreadId = $state(''),
    openingId = '';
  function anchorUnread() {
    const row = layout.find((r) => r.message._id === openingId);
    if (row) {
      scroller.scrollTop = row.offset + 47;
      scrollTop = scroller.scrollTop;
      atBottom = false;
    }
  }
  let beforeLoadHeight = 0;
  async function latest() {
    openingId = '';
    await tick();
    scroller?.scrollTo({ top: scroller.scrollHeight, behavior: 'instant' });
    atBottom = true;
    newCount = 0;
    await markRead({ room: room._id });
  }
  $effect(() => {
    const id = room._id;
    const newest = messages.at(-1)?._id ?? '';
    const oldest = messages[0]?._id ?? '';
    untrack(() => {
      if (id !== previousRoom) {
        previousRoom = id;
        previousNewest = newest;
        previousOldest = oldest;
        measurements = {};
        newCount = 0;
        unreadId = room.unread ? (messages.at(-Math.min(5, room.unread))?._id ?? '') : '';
        if (unreadId) {
          openingId = unreadId;
          atBottom = false;
          void tick().then(anchorUnread);
        } else void latest();
      } else {
        if (newest !== previousNewest) {
          if (atBottom) void latest();
          else newCount++;
          previousNewest = newest;
        }
        if (oldest !== previousOldest && beforeLoadHeight) {
          const delta = total - beforeLoadHeight;
          void tick().then(() => {
            scroller.scrollTop += delta;
          });
          beforeLoadHeight = 0;
        }
        previousOldest = oldest;
      }
    });
  });
  function measure(node: HTMLElement, id: string) {
    const observer = new ResizeObserver(([entry]) => {
      const h = entry.borderBoxSize[0]?.blockSize ?? node.getBoundingClientRect().height;
      if (Math.abs((measurements[id] ?? 0) - h) > 1) {
        const wasBottom = atBottom;
        const row = layout.find((r) => r.message._id === id);
        const delta = h - (measurements[id] ?? row?.size ?? h);
        measurements[id] = h;
        if (openingId) {
          void tick().then(anchorUnread);
          return;
        }
        if (row && row.offset + row.size < scroller.scrollTop) scroller.scrollTop += delta;
        if (wasBottom)
          void tick().then(() => scroller?.scrollTo({ top: scroller.scrollHeight, behavior: 'instant' }));
      }
    });
    observer.observe(node);
    return { destroy: () => observer.disconnect() };
  }
  onMount(() => {
    const read = () => {
      if (!document.hidden) void markRead({ room: room._id }).catch(() => {});
    };
    read();
    document.addEventListener('visibilitychange', read);
    return () => document.removeEventListener('visibilitychange', read);
  });
  $effect(() => {
    const newest = messages.at(-1)?._id;
    if (newest && !document.hidden) void markRead({ room: room._id }).catch(() => {});
  });
  function scroll() {
    scrollTop = scroller.scrollTop;
    atBottom = !openingId && scroller.scrollHeight - scroller.scrollTop - scroller.clientHeight < 65;
    if (atBottom) newCount = 0;
  }
  function older() {
    openingId = '';
    beforeLoadHeight = total;
    query.loadMore(50);
  }
</script>

<div class="chat-view">
  {#if room.voice.length && call.room !== room._id}<div class="room-call-banner">
      <span class="live-dot"></span>
      <div class="avatar-stack">
        {#each room.voice.slice(0, 3) as p}<Avatar
            name={p.name}
            color={p.color}
            size={28}
            speaking={(call.levels[p.user] ?? 0) > 0.5}
          />{/each}
      </div>
      <p>
        <strong
          >{room.voice
            .map((p) => p.name.split(' ')[0])
            .slice(0, 2)
            .join(', ')}{room.voice.length > 2 ? ` +${room.voice.length - 2}` : ''}</strong
        >
        {room.voice.length === 1 ? 'is' : 'are'} in a call
      </p>
      <button class="button primary small" onclick={() => join(room)}
        ><Icon name="headphones" size={14} />Join</button
      >
    </div>{/if}
  {#if isDemo && messages.length}<div class="pinned-strip">
      <Icon name="pin" size={13} /><span>Pinned</span><button
        onclick={() =>
          onThread(
            messages.find((m) => m.attachments.some((a) => a.id === 'direction'))?._id ??
              messages.at(-1)!._id,
          )}>The new direction <Icon name="right" size={12} /></button
      >
    </div>
  {/if}
  <!-- svelte-ignore a11y_no_noninteractive_tabindex, a11y_no_noninteractive_element_interactions (The scrollable log needs keyboard scrolling.) -->
  <div
    class="message-scroll"
    bind:this={scroller}
    bind:clientHeight={viewport}
    onscroll={scroll}
    onwheel={() => (openingId = '')}
    onpointerdown={() => (openingId = '')}
    onkeydown={() => (openingId = '')}
    role="log"
    aria-label={`Messages in ${room.name}`}
    aria-live="off"
    tabindex="0"
  >
    <div class="history-edge">
      {#if query.status === 'CanLoadMore'}<button onclick={older}
          ><Icon name="arrowUp" size={13} />Load earlier messages</button
        >{:else}<span class="room-start">This is the start of {room.kind === 'dm' ? 'your conversation' : `#${room.name}`}.</span
        >{/if}
    </div>
    <div class="virtual-messages" style:height={`${total}px`}>
      {#each visible as row (row.message._id)}<div
          class="virtual-row"
          style:top={`${row.offset}px`}
          use:measure={row.message._id}
        >
          {#if row.day}<div class="day-separator">
              <span
                >{sameDay(row.message._creationTime, Date.now())
                  ? 'Today'
                  : new Date(row.message._creationTime).toLocaleDateString('en', {
                      month: 'long',
                      day: 'numeric',
                    })}</span
              >
            </div>{/if}
          {#if row.message._id === unreadId}<div class="unread-divider">
              <span>New</span>
            </div>{/if}
          <Message message={row.message} grouped={row.grouped} {onThread} />
        </div>{/each}
    </div>
    {#if query.error}<p class="error" role="alert">{errorMessage(query.error)}</p>{/if}
    {#if !messages.length}<div class="empty-state">
        <h2>No messages yet</h2>
        <p>Send the first one below.</p>
      </div>{/if}
  </div>
  {#if !atBottom}<button class="jump-latest" onclick={latest}
      >{newCount ? `${newCount} new message${newCount > 1 ? 's' : ''}` : 'Jump to latest'}<Icon
        name="arrowDown"
        size={15}
      /></button
    >{/if}
  <div class="sr-only" role="status" aria-live="polite" aria-atomic="true">
    {newCount ? `${newCount} new messages in ${room.name}` : ''}
  </div>
  <div class="typing-indicator" aria-live="polite">
    {#if typing.data?.length}<span class="typing-dots"><i></i><i></i><i></i></span><strong
        >{typing.data.map((p) => p.name).join(', ')}</strong
      > {typing.data.length === 1 ? 'is' : 'are'} typing…{/if}
  </div>
  <Composer room={room._id} name={room.name} {space} onSent={latest} {command} />
</div>
