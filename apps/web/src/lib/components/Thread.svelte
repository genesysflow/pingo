<script lang="ts">
  import { api, useQuery, errorMessage } from '../data';
  import Icon from './Icon.svelte';
  import Message from './Message.svelte';
  import Composer from './Composer.svelte';
  let { root, space, close }: { root: string; space: string; close: () => void } = $props();
  const thread = useQuery(api.messages.thread, () => ({ root }));
</script>

<aside class="right-panel thread-panel" aria-label="Thread">
  <div class="panel-heading">
    <div>
      <Icon name="thread" />
      <h2>A little side conversation</h2>
    </div>
    <button class="icon-button" onclick={close} aria-label="Close thread"><Icon name="close" /></button>
  </div>
  <div class="thread-messages">
    {#if thread.data}<Message message={thread.data.root} narrow />
      <div class="day-separator"><span>{thread.data.replies.length} replies</span></div>
      {#each thread.data.replies as message (message._id)}<Message
          {message}
          narrow
        />{/each}{:else if thread.error}<p class="error">{errorMessage(thread.error)}</p>{/if}
  </div>
  {#if thread.data}{#key root}<Composer
        room={thread.data.root.room}
        name="thread"
        {space}
        threadRoot={root}
      />{/key}{/if}
</aside>
