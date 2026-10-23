<script lang="ts">
  import { tick } from 'svelte';
  import { api, useQuery, useAuth, useMutation, type RoomSummary, type SpaceSummary } from '../data';
  import Icon from './Icon.svelte';
  import Modal from './Modal.svelte';
  const auth = useAuth();
  let {
    space,
    rooms,
    spaces,
    close,
    navigate,
    action,
  }: {
    space: string;
    rooms: RoomSummary[];
    spaces: SpaceSummary[];
    close: () => void;
    navigate: (room: string, space?: string) => void;
    action: (name: string) => void;
  } = $props();
  let text = $state(''),
    selected = $state(0);
  const members = useQuery(api.spaces.members, () => ({ space })),
    search = useQuery(api.messages.search, () =>
      text.trim().length > 1 ? { space, text: text.replace(/^search\s+/i, '') } : 'skip',
    ),
    openDm = useMutation(api.dms.open);
  const entries = $derived(
    [
      ...rooms.map((r) => ({
        title: r.name,
        detail: r.kind === 'lounge' ? 'Spatial lounge' : 'Room',
        icon: r.kind === 'lounge' ? ('sprout' as const) : ('hash' as const),
        run: () => navigate(r._id),
      })),
      ...spaces.map((s) => ({
        title: s.name,
        detail: 'Space',
        icon: 'compass' as const,
        run: () => {
          const id =
            s._id === 'fieldwork'
              ? 'design'
              : s._id === 'afterhours'
                ? 'music'
                : s._id === 'opensource'
                  ? 'garden'
                  : '';
          navigate(id, s._id);
        },
      })),
      ...(members.data ?? [])
        .filter((m) => m.user !== auth.userId)
        .map((p) => ({
          title: p.name,
          detail: 'Direct message',
          icon: 'users' as const,
          run: async () => navigate(await openDm({ user: p.user })),
        })),
      ...[
        { title: 'Start a call', action: 'call', icon: 'headphones' as const },
        { title: 'Share screen', action: 'share', icon: 'share' as const },
        { title: 'Toggle mute', action: 'mute', icon: 'mic' as const },
        { title: 'Set status', action: 'status', icon: 'smile' as const },
        { title: 'Change theme', action: 'theme', icon: 'sun' as const },
        { title: 'Sound check', action: 'sound', icon: 'audio' as const },
        { title: 'Catch up', action: 'inbox', icon: 'inbox' as const },
        { title: 'Keyboard shortcuts', action: 'shortcuts', icon: 'keyboard' as const },
      ].map((a) => ({ title: a.title, detail: 'Make it happen', icon: a.icon, run: () => action(a.action) })),
    ].filter((e) => e.title.toLowerCase().includes(text.toLowerCase())),
  );
  const results = $derived([
    ...entries.slice(0, text ? 15 : 8),
    ...(search.data ?? [])
      .slice(0, 8)
      .map((m) => ({
        title: m.body.slice(0, 100),
        detail: `${m.author.name} · message`,
        icon: 'thread' as const,
        run: () => navigate(m.room),
      })),
  ]);
  async function run(i: number) {
    const entry = results[i];
    if (entry) {
      await entry.run();
      close();
    }
  }
  function keydown(e: KeyboardEvent) {
    if (['ArrowDown', 'ArrowUp'].includes(e.key)) {
      e.preventDefault();
      selected = (selected + (e.key === 'ArrowDown' ? 1 : -1) + results.length) % Math.max(results.length, 1);
      void tick().then(() =>
        document.getElementById(`command-${selected}`)?.scrollIntoView({ block: 'nearest' }),
      );
    }
    if (e.key === 'Enter') {
      e.preventDefault();
      void run(selected);
    }
  }
</script>

<Modal title="Find your way" {close}>
  <div class="palette-search">
    <Icon name="search" size={22} /><input
      aria-label="Search commands, rooms, people and messages"
      placeholder="Where would you like to go?"
      bind:value={text}
      oninput={() => (selected = 0)}
      onkeydown={keydown}
      role="combobox"
      aria-expanded="true"
      aria-controls="command-results"
      aria-activedescendant={`command-${selected}`}
      autocomplete="off"
    /><kbd>esc</kbd>
  </div>
  <div class="palette-section-label">{text ? 'A FEW GOOD MATCHES' : 'YOUR NEXT LITTLE MOVE'}</div>
  <div class="command-results" id="command-results" role="listbox">
    {#each results as r, i}<button
        id={`command-${i}`}
        role="option"
        aria-selected={selected === i}
        class:chosen={selected === i}
        onclick={() => run(i)}
        onpointermove={() => (selected = i)}
        ><span class="command-icon"><Icon name={r.icon} /></span><span
          ><strong>{r.title}</strong><small>{r.detail}</small></span
        >{#if selected === i}<Icon name="enter" size={16} />{/if}</button
      >{/each}{#if !results.length}<p class="search-empty">
        No matches yet. Try a name, room, or a few words.
      </p>{/if}
  </div>
  <div class="palette-footer">
    <span><kbd>↑</kbd><kbd>↓</kbd> to explore</span><span><kbd>↵</kbd> to go</span><span
      >Good things are a few keys away.</span
    >
  </div>
</Modal>
