<script lang="ts">
  import { api, useQuery, useMutation, errorMessage, type AudioProfile } from '../data';
  import Modal from './Modal.svelte';
  let { room, close, removed }: { room: string; close: () => void; removed: () => void } = $props();
  const info = useQuery(api.rooms.get, () => ({ room }));
  const update = useMutation(api.rooms.update),
    remove = useMutation(api.rooms.remove);
  let name = $state(''),
    topic = $state(''),
    profile = $state<AudioProfile>('voice'),
    position = $state(0),
    error = $state(''),
    confirm = $state(false);
  $effect(() => {
    if (info.data) {
      name = info.data.name;
      topic = info.data.topic;
      profile = info.data.audioProfile;
      position = info.data.position;
    }
  });
  async function save(e: SubmitEvent) {
    e.preventDefault();
    error = '';
    try {
      await update({ room, name, topic, audioProfile: profile, position });
      close();
    } catch (e) {
      error = errorMessage(e);
    }
  }
</script>

<Modal title="Room settings" {close}>
  <form onsubmit={save}>
    <label class="field">Room name<input required maxlength="32" bind:value={name} /></label>
    <label class="field">Topic<input bind:value={topic} /></label>
    <label class="field"
      >Call audio<select bind:value={profile}
        ><option value="voice">Voice · clear conversation</option><option value="studio"
          >Studio · stereo music, no microphone processing</option
        ></select
      ></label
    >
    <label class="field">Position<input type="number" bind:value={position} required /></label>
    <p class="muted tiny">Audio profile changes apply when you next join the call.</p>
    <button class="button primary">Save room</button>
  </form>
  {#if confirm}<p>Delete this room and its conversations?</p>
    <button
      class="button danger"
      onclick={async () => {
        try {
          await remove({ room });
          removed();
          close();
        } catch (e) {
          error = errorMessage(e);
        }
      }}>Delete room</button
    ><button class="text-button" onclick={() => (confirm = false)}>Keep room</button>
  {:else}<button class="text-button" onclick={() => (confirm = true)}>Delete this room</button>{/if}
  {#if error}<p class="error" role="alert">{error}</p>{/if}
</Modal>
