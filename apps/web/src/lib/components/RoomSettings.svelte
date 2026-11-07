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
      ><small class="field-hint">Applies the next time you join the call.</small></label
    >
    <label class="field">Position<input type="number" bind:value={position} required /></label>
    <button class="button primary full">Save room</button>
  </form>
  <div class="danger-zone">
    {#if confirm}<p>Delete this room and its conversations?</p>
      <div class="inline-actions">
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
        ><button class="button" onclick={() => (confirm = false)}>Keep room</button>
      </div>
    {:else}<button class="button small danger" onclick={() => (confirm = true)}>Delete this room</button>{/if}
  </div>
  {#if error}<p class="error" role="alert">{error}</p>{/if}
</Modal>
