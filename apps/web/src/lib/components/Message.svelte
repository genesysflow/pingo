<script lang="ts">
  import { errorMessage } from '../data/errors';
  import { api, useMutation, useAuth, isDemo, type MessageView } from '../data';
  import { emoji, formatBytes, formatTime } from '../logic/chat';
  import Avatar from './Avatar.svelte';
  import Icon from './Icon.svelte';
  import Markdown from './Markdown.svelte';
  const auth = useAuth();
  let {
    message: m,
    grouped = false,
    onThread,
    narrow = false,
  }: {
    message: MessageView;
    grouped?: boolean;
    onThread?: (id: string) => void;
    narrow?: boolean;
  } = $props();
  let editing = $state(false),
    editBody = $state(''),
    picker = $state(false),
    deleting = $state(false),
    error = $state('');
  const react = useMutation(api.reactions.toggle),
    edit = useMutation(api.messages.edit),
    remove = useMutation(api.messages.remove);
  async function save() {
    try {
      await edit({ message: m._id, body: editBody });
      editing = false;
    } catch (e) {
      error = errorMessage(e);
    }
  }
</script>

<article
  data-message-id={m._id}
  class="message"
  class:grouped
  class:narrow
  class:deleted={m.deleted}
  aria-label={`${m.author.name} at ${formatTime(m._creationTime)}`}
>
  {#if !grouped}<Avatar
      url={m.author.avatarUrl}
      name={m.author.name}
      color={m.author.color}
      size={narrow ? 30 : 36}
    />{:else}<span class="group-time">{formatTime(m._creationTime)}</span>{/if}
  <div class="message-body">
    {#if !grouped}<div class="message-author">
        <strong>{m.author.name}</strong>{#if m.author.user === auth.userId}<span class="you-tag">you</span
          >{/if}<time datetime={new Date(m._creationTime).toISOString()}>{formatTime(m._creationTime)}</time>
      </div>{/if}
    {#if m.deleted}<p class="muted"><i>This message was deleted.</i></p>
    {:else if editing}
      <textarea
        class="edit-input"
        bind:value={editBody}
        aria-label="Edit message"
        maxlength="4000"
        onkeydown={(e) => {
          if (e.key === 'Escape') editing = false;
        }}></textarea>
      <div class="inline-actions">
        <button class="button small" onclick={() => (editing = false)}>Cancel</button><button
          class="button primary small"
          onclick={save}>Save changes</button
        >
      </div>
    {:else}
      <Markdown text={m.body} />{#if m.editedAt}<span class="muted tiny">edited</span>{/if}
      {#each m.attachments as f (f.id)}
        {#if f.contentType.startsWith('image/')}<a
            class="image-attachment"
            href={f.url}
            target="_blank"
            rel="noreferrer"
            ><img src={f.url} alt={f.name} width={f.width} height={f.height} loading="lazy" /><span
              ><Icon name="file" size={14} />{f.name}<small>{formatBytes(f.size)}</small><Icon
                name="upRight"
                size={14}
              /></span
            ></a
          >
        {:else}<a class="file-attachment" href={f.url} download={f.name}
            ><Icon name="file" size={24} /><span
              ><strong>{f.name}</strong><small>{formatBytes(f.size)}</small></span
            ><Icon name="download" /></a
          >{/if}
      {/each}
      {#if m.reactions.length}<div class="reactions">
          {#each m.reactions as r}<button
              class:mine={r.mine}
              aria-pressed={r.mine}
              aria-label={`${r.emoji}, ${r.count} reactions`}
              onclick={() =>
                react({ message: m._id, emoji: r.emoji }).catch((e) => (error = errorMessage(e)))}
              ><span>{r.emoji}</span>{r.count}</button
            >{/each}<button class="add-reaction" aria-label="Add reaction" onclick={() => (picker = !picker)}
            ><Icon name="smile" size={15} /><span>+</span></button
          >
        </div>{/if}
      {#if m.replyCount && onThread}<button class="thread-link" onclick={() => onThread?.(m._id)}
          >{#if isDemo}<span class="tiny-avatars"
              ><Avatar name="Leo Park" color="#a0c4af" size={19} /><Avatar
                name="Sofia Reyes"
                color="#e6a4ba"
                size={19}
              /><Avatar name="Alex Morgan" color="#b9a2e7" size={19} /></span
            >{/if}{m.replyCount} replies<span class="muted">View thread</span><Icon
            name="right"
            size={13}
          /></button
        >{/if}
    {/if}
    {#if error}<p class="error" role="alert">{error}</p>{/if}
    {#if deleting}<div class="inline-confirm">
        Delete this message?<button
          class="button small danger"
          onclick={async () => {
            try {
              await remove({ message: m._id });
              deleting = false;
            } catch (e) {
              error = errorMessage(e);
            }
          }}>Delete</button
        ><button class="button small" onclick={() => (deleting = false)}>Cancel</button>
      </div>{/if}
    {#if picker}<div class="emoji-picker" role="group" aria-label="Choose a reaction">
        {#each Object.values(emoji) as em}<button
            aria-label={`React ${em}`}
            onclick={async () => {
              try {
                await react({ message: m._id, emoji: em });
                picker = false;
              } catch (e) {
                error = errorMessage(e);
              }
            }}>{em}</button
          >{/each}<button aria-label="Close reactions" onclick={() => (picker = false)}
          ><Icon name="close" size={15} /></button
        >
      </div>{/if}
  </div>
  {#if !m.deleted && !editing}<div class="message-tools">
      <button aria-label="React to message" onclick={() => (picker = !picker)}
        ><Icon name="smile" size={16} /></button
      >{#if onThread}<button aria-label="Reply in thread" onclick={() => onThread?.(m._id)}
          ><Icon name="thread" size={16} /></button
        >{/if}{#if m.author.user === auth.userId}<button
          aria-label="Edit message"
          onclick={() => {
            editBody = m.body;
            editing = true;
          }}><Icon name="edit" size={16} /></button
        ><button aria-label="Delete message" onclick={() => (deleting = true)}
          ><Icon name="trash" size={16} /></button
        >{/if}
    </div>{/if}
</article>
