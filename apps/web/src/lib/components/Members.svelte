<script lang="ts">
  import { api, useQuery, useMutation } from '../data';
  import type { CallController } from '../call/controller.svelte';
  import Avatar from './Avatar.svelte';
  import Icon from './Icon.svelte';
  let { space, call, close, openDm }: { space: string; call: CallController; close: () => void; openDm: (id: string) => void } = $props();
  const members = useQuery(api.spaces.members, () => ({ space })), dm = useMutation(api.dms.open);
</script>
<aside class="right-panel member-panel" aria-label="Members"><div class="panel-heading"><div><Icon name="users"/><h2>Your people</h2></div><button class="icon-button" onclick={close} aria-label="Close members"><Icon name="close"/></button></div><p class="panel-note">Good things happen with good company.</p>
{#each ['online', 'idle', 'dnd', 'offline'] as status}{@const group = members.data?.filter(p => p.status === status) ?? []}{#if group.length}<div class="member-group-label">{status === 'dnd' ? 'In the flow' : status === 'idle' ? 'A little away' : status}<span>{group.length}</span></div>{#each group as p}<div class="member-row"><button onclick={async () => openDm(await dm({ user: p.user }))}><Avatar name={p.name} color={p.color} status={p.status} speaking={(call.levels[p.user] ?? 0) > .5 && !!call.room}/><span><strong>{p.name}</strong><small>{p.roles.includes('maker') ? 'Maker · that’s you' : 'Here for the good ideas'}</small></span></button>{#if call.room}<label class="volume-control"><span class="sr-only">Volume for {p.name}</span><input type="range" min="0" max="1.5" step="0.05" value={call.volumes[p.user]?.volume ?? 1} oninput={e => call.setParticipantVolume(p.user, +e.currentTarget.value)}/></label>{/if}</div>{/each}{/if}{/each}
</aside>
