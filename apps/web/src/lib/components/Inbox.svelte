<script lang="ts">
  import { api, useQuery, useMutation, errorMessage, isDemo, type InboxItem } from '../data';
  import { formatTime } from '../logic/chat';
  import Avatar from './Avatar.svelte';
  import Markdown from './Markdown.svelte';
  import Icon from './Icon.svelte';
  let { open }: { open: (room: string, thread?: string) => void } = $props();
  const items = useQuery(api.inbox.list),
    count = useQuery(api.inbox.count),
    done = useMutation(api.inbox.markDone),
    allDone = useMutation(api.inbox.markAllDone);
  let filter = $state('All'),
    selected = $state(0),
    error = $state('');
  const filtered = $derived(
    (items.data ?? []).filter(
      (i) =>
        filter === 'All' ||
        (filter === 'Mentions'
          ? i.kind === 'mention'
          : filter === 'Replies'
            ? i.kind === 'reply'
            : i.kind === 'dm'),
    ),
  );
  function jump(i: InboxItem) {
    open(i.message.room, i.kind === 'reply' ? i.message.threadRoot : undefined);
  }
  function keydown(e: KeyboardEvent) {
    if (
      e.target instanceof HTMLInputElement ||
      e.target instanceof HTMLTextAreaElement ||
      document.querySelector('[role="dialog"]')
    )
      return;
    if (['j', 'k'].includes(e.key)) {
      e.preventDefault();
      selected = Math.max(0, Math.min(filtered.length - 1, selected + (e.key === 'j' ? 1 : -1)));
      document.getElementById(`inbox-${selected}`)?.focus();
    }
    if (e.key === 'e' && filtered[selected])
      void done({ item: filtered[selected]._id }).catch((e) => (error = errorMessage(e)));
  }
</script>

<svelte:window onkeydown={keydown} />
<div class="inbox-view">
  <div class="inbox-filters">
    <div role="tablist" aria-label="Inbox filter">
      {#each ['All', 'Mentions', 'Replies', 'Direct messages'] as f}<button
          class:active={filter === f}
          role="tab"
          aria-selected={filter === f}
          onclick={() => {
            filter = f;
            selected = 0;
          }}
          >{f}{#if f === 'All' && count.data}<span>{count.data}</span>{/if}</button
        >{/each}
    </div>
    <button
      class="text-button"
      disabled={!count.data}
      onclick={() => allDone().catch((e) => (error = errorMessage(e)))}
      ><Icon name="checkAll" size={16} />Mark all done</button
    >
  </div>
  {#if error || items.error}<p class="error" role="alert">{error || errorMessage(items.error)}</p>{/if}
  <div class="inbox-feed">
    {#each filtered as item, i (item._id)}<article class="inbox-card" class:selected={selected === i}>
        <div class="inbox-card-top">
          <span class="inbox-kind {item.kind}"
            ><Icon
              name={item.kind === 'mention' ? 'bell' : item.kind === 'reply' ? 'thread' : 'mail'}
              size={13}
            />{item.kind === 'mention'
              ? 'Mention'
              : item.kind === 'reply'
                ? 'Thread reply'
                : 'Direct message'}</span
          ><span
            >{item.room.kind === 'dm'
              ? item.message.author.name
              : `${item.space?.name ?? ''} · #${item.room.name}`}<time>{formatTime(item.message._creationTime)}</time></span
          >
        </div>
        <div class="inbox-card-body">
          <Avatar
            url={item.message.author.avatarUrl}
            name={item.message.author.name}
            color={item.message.author.color}
            size={38}
          />
          <div>
            <strong>{item.message.author.name}</strong><Markdown
              text={item.message.body}
            />{#if item.message.attachments.length}<span class="attachment-label"
                ><Icon name="attach" size={13} />{item.message.attachments[0].name}</span
              >{/if}
          </div>
        </div>
        <div class="inbox-card-actions">
          <button
            class="text-button"
            id={`inbox-${i}`}
            onfocus={() => (selected = i)}
            onclick={() => jump(item)}
            >{item.kind === 'reply' ? 'Open thread' : 'Open'}<Icon
              name="arrowRight"
              size={14}
            /></button
          ><button
            class="button small"
            onclick={() => done({ item: item._id }).catch((e) => (error = errorMessage(e)))}
            ><Icon name="check" size={14} />Done</button
          >
        </div>
      </article>{/each}
    {#if !filtered.length}<div class="empty-state">
        <h2>{filter === 'All' ? 'You’re all caught up' : 'Nothing here'}</h2>
        <p>
          {filter === 'All'
            ? 'Mentions, thread replies and direct messages land here.'
            : `No ${filter.toLowerCase()} to review.`}
        </p>
        <button class="button" onclick={() => open(isDemo ? 'design' : '')}>Back to rooms</button>
      </div>{/if}
  </div>
  <div class="inbox-footer">
    <span><kbd>j</kbd><kbd>k</kbd> move · <kbd>e</kbd> done</span>
  </div>
</div>
