<script lang="ts">
  import Flourish from './Flourish.svelte';
  import { api, useQuery, useMutation, type InboxItem } from '../data';
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
    selected = $state(0);
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
    if (e.key === 'e' && filtered[selected]) void done({ item: filtered[selected]._id });
  }
</script>

<svelte:window onkeydown={keydown} />
<div class="inbox-view">
  <div class="inbox-intro">
    <div class="eyebrow"><span class="little-star"><Flourish /></span> A MOMENT TO RECONNECT</div>
    <h1>A little catching up.</h1>
    <p>
      {count.data
        ? `${count.data} things for you. Take them one at a time.`
        : 'You’re all caught up. Go make something lovely.'}
    </p>
    <span class="inbox-doodle">✺<i>you’re<br />right on time.</i></span>
  </div>
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
    <button class="text-button" disabled={!count.data} onclick={() => allDone()}
      ><Icon name="checkAll" size={16} />Mark all done</button
    >
  </div>
  <div class="inbox-feed">
    {#each filtered as item, i (item._id)}<article class="inbox-card" class:selected={selected === i}>
        <div class="inbox-card-top">
          <span class="inbox-kind {item.kind}"
            ><Icon
              name={item.kind === 'mention' ? 'bell' : item.kind === 'reply' ? 'thread' : 'mail'}
              size={13}
            />{item.kind === 'mention'
              ? 'Mentioned you'
              : item.kind === 'reply'
                ? 'Replied to your thread'
                : 'A message for you'}</span
          ><span
            >{item.space?.name ?? 'Just the two of you'}<span class="dot-separator">·</span>{item.room
              .kind === 'dm'
              ? 'Direct message'
              : `# ${item.room.name}`}<time>{formatTime(item.message._creationTime)}</time></span
          >
        </div>
        <div class="inbox-card-body">
          <Avatar name={item.message.author.name} color={item.message.author.color} size={38} />
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
            >{item.kind === 'reply' ? 'Open thread' : 'Go to conversation'}<Icon
              name="arrowRight"
              size={14}
            /></button
          ><button class="button small" onclick={() => done({ item: item._id })}
            ><Icon name="check" size={14} />Done</button
          >
        </div>
      </article>{/each}
    {#if !filtered.length}<div class="empty-state">
        <span class="caught-up-flower">✺</span>
        <h2>Room to breathe.</h2>
        <p>
          {filter === 'All'
            ? 'Nothing needs your attention. Enjoy the quiet.'
            : 'Nothing here right now. A little less to think about.'}
        </p>
        <button class="button soft" onclick={() => open('design')}
          >Back to your people<Icon name="arrowRight" size={15} /></button
        >
      </div>{/if}
  </div>
  <div class="inbox-footer">
    <Icon name="leaf" size={14} />You don’t have to be everywhere, all at once.<span
      ><kbd>j</kbd><kbd>k</kbd> to browse · <kbd>e</kbd> to mark done</span
    >
  </div>
</div>
