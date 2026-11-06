<script lang="ts">
  import { onDestroy } from 'svelte';
  import { api, useQuery, useMutation, errorMessage } from '../data';
  import { createReadObserver } from '../logic/read-messages';
  import Icon from './Icon.svelte';
  import Message from './Message.svelte';
  import Composer from './Composer.svelte';
  let { root, space, close }: { root: string; space: string; close: () => void } = $props();
  const thread = useQuery(api.messages.thread, () => ({ root }));
  const markSeen = useMutation(api.inbox.markRead);
  const reads = createReadObserver((messages) => markSeen({ messages }));
  onDestroy(() => reads.destroy());
</script>

<aside class="right-panel thread-panel" aria-label="Thread">
  <div class="panel-heading">
    <div>
      <Icon name="thread" />
      <h2>Thread</h2>
    </div>
    <button class="icon-button" onclick={close} aria-label="Close thread"><Icon name="close" /></button>
  </div>
  <div class="thread-messages">
    {#if thread.data}{#key root}<div use:reads.observe={thread.data.root._id}>
          <Message message={thread.data.root} narrow />
        </div>{/key}
      <div class="day-separator"><span>{thread.data.replies.length} replies</span></div>
      {#each thread.data.replies as message (message._id)}<div use:reads.observe={message._id}>
          <Message {message} narrow />
        </div>{/each}{:else if thread.error}<p class="error">{errorMessage(thread.error)}</p>{/if}
  </div>
  {#if thread.data}{#key root}<Composer
        room={thread.data.root.room}
        name="thread"
        {space}
        threadRoot={root}
      />{/key}{/if}
</aside>
