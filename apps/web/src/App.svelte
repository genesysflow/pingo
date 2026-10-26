<script lang="ts">
  import { errorMessage } from './lib/data/errors';
  import { onMount } from 'svelte';
  import {
    api,
    useQuery,
    useMutation,
    useAuth,
    type RoomSummary,
    type PresenceStatus,
    isDemo,
  } from './lib/data';
  import { initializeData, createCall, startActivity } from './lib/data/runtime';
  import Avatar from './lib/components/Avatar.svelte';
  import Icon from './lib/components/Icon.svelte';
  import Chat from './lib/components/Chat.svelte';
  import Thread from './lib/components/Thread.svelte';
  import Members from './lib/components/Members.svelte';
  import Inbox from './lib/components/Inbox.svelte';
  import Palette from './lib/components/Palette.svelte';
  import CallPill from './lib/components/CallPill.svelte';
  import Stage from './lib/components/Stage.svelte';
  import Lounge from './lib/components/Lounge.svelte';
  import SoundCheck from './lib/components/SoundCheck.svelte';
  import Settings from './lib/components/Settings.svelte';
  import Auth from './lib/components/Auth.svelte';
  import AccessGate from './lib/components/AccessGate.svelte';
  import RoomSettings from './lib/components/RoomSettings.svelte';
  import Modal from './lib/components/Modal.svelte';
  import { nativeDesktop } from './lib/desktop/bridge';
  import DesktopEvents from './lib/desktop/DesktopEvents.svelte';
  import { DEFAULT_ACCENT, inkFor as onAccent } from './lib/theme';
  initializeData();
  const auth = useAuth(),
    call = createCall();
  let path = $state(location.pathname),
    space = $state(location.pathname.match(/^\/s\/([^/]+)/)?.[1] ?? (isDemo ? 'fieldwork' : '')),
    roomId = $state(location.pathname.match(/\/r\/([^/]+)/)?.[1] ?? (isDemo ? 'design' : ''));
  let view = $state<'chat' | 'inbox' | 'settings'>(
    location.pathname === '/catch-up' ? 'inbox' : location.pathname === '/settings' ? 'settings' : 'chat',
  );
  let theme = $state(
      localStorage.getItem('pingo-theme') ??
        (matchMedia('(prefers-color-scheme: dark)').matches ? 'dark' : 'light'),
    ),
    density = $state(localStorage.getItem('pingo-density') ?? 'comfortable');
  let palette = $state(false),
    shortcutSheet = $state(false),
    thread = $state<string | null>(null),
    members = $state(false),
    stage = $state(false),
    sound = $state(false),
    mobileSidebar = $state(false),
    settingsPage = $state('Appearance');
  let toast = $state(''),
    workspaceMenu = $state(false),
    roomSettings = $state(false),
    createPrivate = $state(false),
    modal = $state<'space' | 'room' | 'status' | null>(null),
    createName = $state(''),
    joinCode = $state(''),
    formError = $state(''),
    roomKind = $state<'chat' | 'lounge' | 'stage'>('chat'),
    customStatus = $state(''),
    presenceStatus = $state<PresenceStatus>('online');
  let pendingCall: RoomSummary | null = null,
    toastTimer: ReturnType<typeof setTimeout>;
  // The server's registration policy can keep a signed-in account out until it is invited or verified.
  const access = useQuery(api.access.status);
  const admitted = $derived(auth.isAuthenticated && access.data?.admission === 'admitted');
  const spaces = useQuery(api.spaces.list, () => (admitted ? {} : 'skip')),
    rooms = useQuery(api.rooms.list, () => (admitted && space ? { space } : 'skip')),
    me = useQuery(api.profiles.me),
    dms = useQuery(api.dms.list, () => (admitted ? {} : 'skip')),
    inboxCount = useQuery(api.inbox.count, () => (admitted ? {} : 'skip'));
  const roomInfo = useQuery(api.rooms.get, () =>
    admitted && roomId ? { room: roomId } : 'skip',
  );
  const callInfo = useQuery(api.rooms.get, () => (call.room ? { room: call.room } : 'skip'));
  const currentRoom = $derived({
    data: roomInfo.data
      ? {
          ...roomInfo.data,
          unread: rooms.data?.find((r) => r._id === roomId)?.unread ?? 0,
          mentions: rooms.data?.find((r) => r._id === roomId)?.mentions ?? 0,
          voice: rooms.data?.find((r) => r._id === roomId)?.voice ?? [],
        }
      : undefined,
  });
  const callRoom = $derived({
    data: callInfo.data
      ? {
          ...callInfo.data,
          unread: 0,
          mentions: 0,
          voice: [],
        }
      : undefined,
  });
  const currentSpace = $derived(spaces.data?.find((s) => s._id === space));
  const createSpace = useMutation(api.spaces.create),
    createRoom = useMutation(api.rooms.create),
    acceptInvite = useMutation(api.invites.accept),
    setStatus = useMutation(api.presence.setStatus),
    heartbeat = useMutation(api.presence.heartbeat);
  const inviteCode = $derived(path.match(/^\/invite\/([^/]+)/)?.[1]);
  function route(url: string) {
    history.pushState({}, '', url);
    path = url;
  }
  function notice(text: string) {
    toast = text;
    clearTimeout(toastTimer);
    toastTimer = setTimeout(() => (toast = ''), 5000);
  }
  function navigate(id: string, nextSpace?: string) {
    if (nextSpace) space = nextSpace;
    if (!id) {
      roomId = '';
      route(`/s/${space}`);
      return;
    }
    roomId = id;
    view = 'chat';
    stage = false;
    thread = null;
    members = false;
    mobileSidebar = false;
    route(`/s/${space}/r/${id}`);
  }
  async function selectSpace(id: string) {
    space = id;
    view = 'chat';
    stage = false;
    thread = null;
    members = false;
    mobileSidebar = false;
    roomId = '';
    route(`/s/${id}`);
  }
  // Live queries arrive asynchronously; wait for each selected space's actual room list.
  $effect(() => {
    if (!admitted || !spaces.data) return;
    if (!spaces.data.some((s) => s._id === space)) {
      space = spaces.data[0]?._id ?? '';
      roomId = '';
    }
    if (space && !roomId && rooms.data?.length) navigate(rooms.data[0]._id);
  });
  $effect(() => {
    if (admitted) void heartbeat().catch((e) => notice(errorMessage(e)));
  });

  function catchUp() {
    view = 'inbox';
    stage = false;
    thread = null;
    members = false;
    mobileSidebar = false;
    route('/catch-up');
  }
  function settings(page = 'Appearance') {
    view = 'settings';
    stage = false;
    thread = null;
    members = false;
    settingsPage = page;
    workspaceMenu = false;
    route('/settings');
  }
  function setTheme(value: string) {
    theme = value;
    localStorage.setItem('pingo-theme', value);
  }
  function setDensity(value: string) {
    density = value;
    localStorage.setItem('pingo-density', value);
  }
  async function join(room: RoomSummary) {
    if (call.room === room._id) {
      stage = true;
      view = 'chat';
      return;
    }
    if (!localStorage.getItem('pingo-sound-checked')) {
      pendingCall = room;
      sound = true;
      return;
    }
    try {
      await call.join(room._id, room.name);
      stage = room.kind !== 'lounge';
      if (room.kind === 'lounge') {
        navigate(room._id);
      } else view = 'chat';
    } catch (e) {
      notice(errorMessage(e));
    }
  }
  async function completeSound() {
    sound = false;
    const target = pendingCall;
    pendingCall = null;
    if (target) await join(target);
    else notice('Sound check saved.');
  }
  function action(name: string) {
    if (name === 'call' && currentRoom.data) void join(currentRoom.data);
    if (name === 'share') {
      if (!call.room) {
        if (currentRoom.data) {
          void join(currentRoom.data);
          notice('Join the call, then share your screen.');
        }
      } else {
        if (!call.sharing) call.toggleShare();
        stage = true;
        view = 'chat';
      }
    }
    if (name === 'mute') {
      if (call.room) call.toggleMute();
      else notice('Join a call to use your microphone.');
    }
    if (name === 'status') {
      customStatus = me.data?.statusText ?? '';
      presenceStatus = me.data?.status ?? 'online';
      modal = 'status';
    }
    if (name === 'theme') setTheme(theme === 'light' ? 'dark' : theme === 'dark' ? 'contrast' : 'light');
    if (name === 'sound') sound = true;
    if (name === 'inbox') catchUp();
    if (name === 'shortcuts') shortcutSheet = true;
  }
  function keyboard(e: KeyboardEvent) {
    const typing =
      e.target instanceof HTMLInputElement ||
      e.target instanceof HTMLTextAreaElement ||
      (e.target instanceof HTMLElement && e.target.isContentEditable);
    if ((e.ctrlKey || e.metaKey) && e.key.toLowerCase() === 'k') {
      e.preventDefault();
      palette = !palette;
    }
    if (e.key === '?' && !typing) {
      e.preventDefault();
      shortcutSheet = !shortcutSheet;
    }
    if (e.key === 'Escape' && !document.querySelector('[role="dialog"]')) {
      thread = null;
      members = false;
      workspaceMenu = false;
      mobileSidebar = false;
      if (stage) stage = false;
    }
  }
  async function submitCreate(e: SubmitEvent) {
    e.preventDefault();
    formError = '';
    try {
      if (modal === 'space') {
        if (joinCode.trim()) {
          const code = joinCode.trim().split('/').at(-1)!;
          const s = await acceptInvite({ code });
          await selectSpace(s);
        } else {
          const s = await createSpace({ name: createName });
          await selectSpace(s);
        }
      } else if (modal === 'room') {
        const id = await createRoom({ space, name: createName, kind: roomKind, private: createPrivate });
        navigate(id);
      }
      modal = null;
      createName = '';
      joinCode = '';
    } catch (e) {
      formError = errorMessage(e);
    }
  }
  $effect(() => {
    if (!auth.isAuthenticated && call.room) call.leave();
  });
  onMount(() => {
    void auth
      .handleRedirectCallback()
      .then((result) => {
        if (result?.type === 'signedIn') {
          if (inviteCode) route(`/invite/${inviteCode}`);
          else navigate(roomId);
        } else if (result?.type === 'error') notice(result.error.message);
      })
      .catch((e) => notice(errorMessage(e)));
    const stop = startActivity();
    const interval = setInterval(() => {
      if (admitted) void heartbeat().catch((e) => notice(errorMessage(e)));
    }, 60_000);
    return () => {
      stop();
      clearInterval(interval);
      clearTimeout(toastTimer);
      call.destroy();
    };
  });
  const accent = $derived(currentSpace?.accent ?? DEFAULT_ACCENT);
  const presenceLabel: Record<string, string> = {
    online: 'Online',
    idle: 'Away',
    dnd: 'Do not disturb',
    offline: 'Invisible',
  };
  function initials(name: string) {
    return name
      .split(/\s+/)
      .filter(Boolean)
      .slice(0, 2)
      .map((w) => w[0]!.toUpperCase())
      .join('');
  }
</script>

{#if nativeDesktop}<DesktopEvents {call} />{/if}

<svelte:window
  onkeydown={keyboard}
  onpopstate={() => {
    path = location.pathname;
    const r = path.match(/^\/s\/([^/]+)\/r\/([^/]+)/);
    if (r) {
      space = r[1];
      roomId = r[2];
      view = 'chat';
    } else view = path === '/catch-up' ? 'inbox' : path === '/settings' ? 'settings' : 'chat';
    stage = false;
  }}
/>
<svelte:head
  ><title
    >{inviteCode
      ? 'Invitation'
      : !auth.isAuthenticated || path === '/signin'
        ? 'Sign in'
        : view === 'inbox'
          ? 'Inbox'
          : view === 'settings'
            ? 'Settings'
            : `${currentRoom.data?.name ?? 'Pingo'}`} · Pingo</title
  ></svelte:head
>
<div
  class="app-root"
  data-theme={theme}
  data-density={density}
  style:--accent={accent}
  style:--on-accent={onAccent(accent)}
>
  {#if !auth.isAuthenticated || inviteCode || path === '/signin' || path === '/signup'}<Auth
      {inviteCode}
      enter={(s) => {
        if (isDemo) navigate('design', s ?? 'fieldwork');
        else {
          roomId = '';
          if (s) space = s;
          route(s ? `/s/${s}` : '/');
        }
      }}
    />
  {:else if access.error}<main class="empty-state">
      <p role="alert">{errorMessage(access.error)}</p>
      <button class="button" onclick={() => auth.signOut()}>Sign out</button>
    </main>
  {:else if !access.data?.admission || (admitted && spaces.isLoading)}<main class="empty-state" aria-busy="true"
      >Loading…</main
    >
  {:else if !admitted}<AccessGate status={access.data} joined={(s) => selectSpace(s)} />
  {:else if spaces.error}<main class="empty-state">
      <p role="alert">{errorMessage(spaces.error)}</p>
      <button class="button" onclick={() => auth.signOut()}>Sign out</button>
    </main>
  {:else if spaces.data?.length === 0}<main class="onboarding">
      <form
        class="onboarding-card"
        onsubmit={(e) => {
          modal = 'space';
          void submitCreate(e);
        }}
      >
        <img src="/pingo.svg" width="36" height="36" alt="Pingo" />
        <h1>Create your first space</h1>
        <p class="settings-description">A space holds your rooms, calls and people.</p>
        <label class="field"
          >Space name<input bind:value={createName} maxlength="32" required={!joinCode} /></label
        >
        <div class="auth-divider"><span>or join one</span></div>
        <label class="field">Invite link or code<input bind:value={joinCode} /></label>
        {#if formError}<p class="error" role="alert">{formError}</p>{/if}
        <button class="button primary full">{joinCode ? 'Join space' : 'Create space'}</button>
        <button type="button" class="text-button" onclick={() => auth.signOut()}>Sign out</button>
      </form>
    </main>
  {:else}
    <a href="#main-content" class="skip-link">Skip to conversation</a>
    <nav class="space-rail" aria-label="Spaces">
      <button
        class="pingo-mark"
        aria-label="Pingo home"
        onclick={() => {
          roomId = '';
          route('/');
        }}><img src="/pingo.svg" alt="Pingo" /></button
      >{#each spaces.data ?? [] as s}<button
          class="space-icon"
          class:active={space === s._id}
          style:--space-color={s.accent}
          style:--space-ink={onAccent(s.accent)}
          aria-label={`${s.name}${s.mentions ? `, ${s.mentions} mentions` : ''}`}
          aria-current={space === s._id ? 'page' : undefined}
          title={s.name}
          onclick={() => selectSpace(s._id)}
          >{#if s.iconUrl}<img src={s.iconUrl} alt="" />{:else}{initials(s.name)}{/if}{#if s.mentions}<span
              class="rail-badge">{s.mentions}</span
            >{:else if s.unread}<span class="rail-unread"></span>{/if}</button
        >{/each}<button
        class="rail-add"
        aria-label="Create or join a space"
        title="Create or join a space"
        onclick={() => {
          modal = 'space';
          createName = '';
          joinCode = '';
          formError = '';
        }}><Icon name="plus" size={18} /></button
      >
      <div class="rail-bottom">
        <button
          class="icon-button"
          aria-label="Keyboard shortcuts"
          title="Keyboard shortcuts"
          onclick={() => (shortcutSheet = true)}><Icon name="keyboard" size={18} /></button
        >
      </div>
    </nav>
    <aside class="sidebar" class:mobile-open={mobileSidebar} aria-label="Rooms and direct messages">
      <div class="space-heading">
        <button
          class="space-heading-main"
          aria-expanded={workspaceMenu}
          onclick={() => (workspaceMenu = !workspaceMenu)}
          ><span class="space-wordmark">{currentSpace?.name ?? 'Space'}</span><Icon
            name="down"
            size={15}
          /></button
        ><button
          class="icon-button mobile-close"
          aria-label="Close sidebar"
          onclick={() => (mobileSidebar = false)}><Icon name="close" /></button
        >{#if workspaceMenu}<div class="workspace-menu">
            <button onclick={() => settings('Space')}
              ><Icon name="settings" size={15} />Space settings & invites</button
            ><button
              onclick={() => {
                modal = 'space';
                workspaceMenu = false;
              }}><Icon name="plus" size={15} />Create or join a space</button
            ><button onclick={() => settings('Profile')}><Icon name="users" size={15} />Your profile</button>
          </div>{/if}
      </div>
      <div class="sidebar-tools">
        <button class="quick-find" onclick={() => (palette = true)}
          ><Icon name="search" size={15} /><span>Search</span><kbd>Ctrl K</kbd></button
        ><button
          class="nav-row"
          class:active={view === 'inbox'}
          aria-label="Inbox"
          aria-current={view === 'inbox' ? 'page' : undefined}
          onclick={catchUp}
          ><Icon name="inbox" size={16} /><span>Inbox</span>{#if inboxCount.data}<span class="count"
              >{inboxCount.data}</span
            >{/if}</button
        >
      </div>
      <div class="sidebar-scroll">
        <div class="section-label">
          <span>Rooms</span><button
            aria-label="Create a room"
            title="Create a room"
            onclick={() => {
              modal = 'room';
              createName = '';
              formError = '';
            }}><Icon name="plus" size={14} /></button
          >
        </div>
        <nav class="room-list" aria-label="Chat rooms">
          {#each (rooms.data ?? []).filter((r) => r.kind === 'chat' || r.kind === 'stage') as r}<div
              class="room-group"
            >
              <button
                class="room-link"
                class:active={roomId === r._id && view === 'chat' && !stage}
                class:unread={r.unread > 0}
                aria-current={roomId === r._id && view === 'chat' && !stage ? 'page' : undefined}
                onclick={() => navigate(r._id)}
                ><Icon name={r.kind === 'stage' ? 'radio' : r.private ? 'lock' : 'hash'} size={15} /><span
                  >{r.name}</span
                >{#if r.mentions}<span class="count accent">{r.mentions}</span>{:else if r.unread}<span
                    class="unread-dot"
                  ></span>{/if}</button
              >{#if r.voice.length}<div class="room-voice">
                  <button
                    class="voice-people"
                    aria-label={`${r.voice.length} people in ${r.name} call`}
                    onclick={() => join(r)}
                    >{#each r.voice.slice(0, 4) as p}<span
                        class="voice-person"
                        class:speaking={(call.levels[p.user] ?? 0) > 0.5 && !p.muted}
                        ><Avatar name={p.name} color={p.color} size={18} /><span>{p.name.split(' ')[0]}</span
                        >{#if p.muted}<Icon name="micOff" size={11} />{/if}</span
                      >{/each}{#if r.voice.length > 4}<span class="voice-more">+{r.voice.length - 4}</span
                      >{/if}</button
                  >{#if call.room !== r._id}<button class="hop-in" onclick={() => join(r)}>Join</button>{/if}
                </div>{/if}
            </div>{/each}
        </nav>
        {#if (rooms.data ?? []).some((r) => r.kind === 'lounge')}<div class="section-label">
            <span>Lounges</span>
          </div>
          <nav class="room-list" aria-label="Lounges">
            {#each (rooms.data ?? []).filter((r) => r.kind === 'lounge') as r}<button
                class="room-link lounge-link"
                class:active={roomId === r._id && view === 'chat'}
                aria-current={roomId === r._id && view === 'chat' ? 'page' : undefined}
                onclick={() => navigate(r._id)}
                ><Icon name="radio" size={15} /><span>{r.name}</span>{#if r.voice.length}<span class="live-count"
                    ><span class="live-dot"></span>{r.voice.length}</span
                  >{/if}</button
              >{/each}
          </nav>{/if}
        <div class="section-label">
          <span>Direct messages</span><button
            aria-label="New direct message"
            title="New direct message"
            onclick={() => {
              palette = true;
            }}><Icon name="plus" size={14} /></button
          >
        </div>
        <nav class="dm-list" aria-label="Direct messages">
          {#each dms.data ?? [] as dm}<button
              class="dm-link"
              class:active={roomId === dm.room && view === 'chat'}
              onclick={() => navigate(dm.room)}
              ><Avatar
                url={dm.other.avatarUrl}
                name={dm.other.name}
                color={dm.other.color}
                size={20}
                status={dm.other.status}
              /><span>{dm.other.name}</span>{#if dm.unread}<span class="unread-dot"></span>{/if}</button
            >{/each}
        </nav>
        <button class="invite-people-button" onclick={() => settings('Space')}
          ><Icon name="plus" size={14} />Invite people</button
        >
      </div>
      <div class="sidebar-user">
        <button class="self-avatar" aria-label="Your profile" onclick={() => settings('Profile')}
          ><Avatar
            url={me.data?.avatarUrl}
            name={me.data?.name ?? 'You'}
            color={me.data?.color}
            size={30}
            status={me.data?.status}
          /></button
        ><button class="self-status" onclick={() => action('status')}
          ><strong>{me.data?.name ?? 'You'}</strong><span
            >{me.data?.statusText || presenceLabel[me.data?.status ?? 'online']}</span
          ></button
        ><button class="icon-button" aria-label="Open settings" title="Settings" onclick={() => settings()}
          ><Icon name="settings" size={16} /></button
        >
      </div>
    </aside>
    <main class="main-shell" id="main-content" tabindex="-1">
      <header class="main-header">
        <button
          class="icon-button mobile-menu"
          aria-label="Open rooms"
          onclick={() => (mobileSidebar = !mobileSidebar)}><Icon name="panel" /></button
        >
        <div class="room-heading">
          <span class="header-room-icon"
            ><Icon
              name={view === 'inbox'
                ? 'inbox'
                : view === 'settings'
                  ? 'settings'
                  : stage
                    ? 'headphones'
                    : currentRoom.data?.kind === 'lounge'
                      ? 'radio'
                      : currentRoom.data?.kind === 'dm'
                        ? 'mail'
                        : currentRoom.data?.kind === 'stage'
                          ? 'radio'
                          : 'hash'}
              size={17}
            /></span
          >
          <h1>
            {view === 'inbox'
              ? 'Inbox'
              : view === 'settings'
                ? 'Settings'
                : stage
                  ? call.roomName
                  : (currentRoom.data?.name ?? 'Pingo')}
          </h1>
          {#if view === 'chat' && !stage && currentRoom.data?.topic}<p>{currentRoom.data.topic}</p>{/if}
        </div>
        <div class="header-actions">
          {#if view === 'chat'}{#if currentRoom.data && currentRoom.data.myPerms & 8 && currentRoom.data.kind !== 'dm'}<button
                class="icon-button"
                aria-label="Room settings"
                title="Room settings"
                onclick={() => (roomSettings = true)}><Icon name="settings" size={17} /></button
              >{/if}<button
              class="icon-button"
              aria-label="Search messages"
              title="Search"
              onclick={() => (palette = true)}><Icon name="search" size={17} /></button
            ><button
              class="icon-button"
              aria-label="Show members"
              title="Members"
              aria-pressed={members}
              class:active={members}
              onclick={() => {
                members = !members;
                thread = null;
              }}><Icon name="users" size={17} /></button
            >{#if !stage}<button
                class="header-call-button"
                class:live={!!call.room}
                onclick={() => {
                  if (call.room) {
                    stage = true;
                  } else if (currentRoom.data) void join(currentRoom.data);
                }}
                >{#if call.room}<span class="live-dot"></span>{:else}<Icon
                    name="headphones"
                    size={15}
                  />{/if}<span>{call.room ? 'Back to call' : 'Start a call'}</span></button
              >{/if}{:else}<button
              class="icon-button"
              aria-label="Search"
              title="Search"
              onclick={() => (palette = true)}><Icon name="search" size={17} /></button
            ><button class="icon-button" aria-label="Change theme" title="Theme" onclick={() => action('theme')}
              ><Icon name={theme === 'light' ? 'moon' : 'sun'} size={17} /></button
            >{/if}
        </div>
      </header>
      <div class="main-columns">
        {#if view === 'inbox'}<Inbox
            open={(id, root) => {
              navigate(id);
              if (root) thread = root;
            }}
          />{:else if view === 'settings'}{#key settingsPage}<Settings
              page={settingsPage}
              {theme}
              {density}
              {space}
              {call}
              {setTheme}
              {setDensity}
              back={() => navigate(roomId)}
              sound={() => (sound = true)}
              {notice}
            />{/key}{:else if stage && call.room && callRoom.data}<Stage
            {call}
            room={callRoom.data}
            collapse={() => (stage = false)}
            {notice}
          />{:else if currentRoom.data?.kind === 'lounge'}<Lounge
            room={currentRoom.data}
            {call}
            {join}
          />{:else if currentRoom.data}{#key currentRoom.data._id}<Chat
              room={currentRoom.data}
              {space}
              {call}
              onThread={(id) => {
                thread = id;
                members = false;
              }}
              {join}
              command={action}
            />{/key}{:else}<div class="empty-state">
            <h2>No room selected</h2>
            <p>Pick a room from the sidebar.</p>
          </div>{/if}
        {#if view === 'chat' && thread}<Thread
            root={thread}
            {space}
            close={() => (thread = null)}
          />{:else if view === 'chat' && members}<Members
            {space}
            {call}
            close={() => (members = false)}
            openDm={(id) => navigate(id)}
          />{/if}
      </div>
    </main>
    {#if call.room && !(stage && view === 'chat')}<CallPill
        {call}
        self={me.data}
        expand={() => {
          stage = true;
          view = 'chat';
        }}
        {notice}
      />{/if}
  {/if}
  {#if palette && admitted}<Palette
      {space}
      rooms={rooms.data ?? []}
      spaces={spaces.data ?? []}
      close={() => (palette = false)}
      {navigate}
      {action}
    />{/if}
  {#if shortcutSheet}<Modal title="Keyboard shortcuts" close={() => (shortcutSheet = false)}
      ><div class="shortcut-list">
        {#each [['Search', 'Ctrl / ⌘ K'], ['Show shortcuts', '?'], ['Send a message', 'Enter'], ['New line', 'Shift Enter'], ['Bold / italic', 'Ctrl / ⌘ B / I'], ['Browse the inbox', 'J / K'], ['Mark inbox item done', 'E'], ['Move in a lounge', '← ↑ ↓ →'], ['Point / draw on a share', 'Enter / D'], ['Close a panel', 'Esc']] as [label, key]}<div
          >
            <span>{label}</span><kbd>{key}</kbd>
          </div>{/each}
      </div></Modal
    >{/if}
  {#if sound}<SoundCheck
      {call}
      close={() => {
        sound = false;
        pendingCall = null;
      }}
      complete={completeSound}
    />{/if}
  {#if (modal === 'space' || modal === 'room') && spaces.data?.length}<Modal
      title={modal === 'space' ? 'Create or join a space' : 'New room'}
      close={() => (modal = null)}
      ><form onsubmit={submitCreate}>
        <label class="field"
          >{modal === 'space' ? 'Space name' : 'Room name'}<input
            bind:value={createName}
            maxlength="32"
            placeholder={modal === 'space' ? 'Acme' : 'design-review'}
            required={!joinCode}
          /></label
        >{#if modal === 'space'}<div class="auth-divider"><span>or join one</span></div>
          <label class="field"
            >Invite link or code<input bind:value={joinCode} placeholder="Paste an invite link" /></label
          >{:else}<label class="field"
            >Type<select bind:value={roomKind}
              ><option value="chat">Chat · messages, with a call when you need one</option><option value="lounge"
                >Lounge · spatial audio, walk up to talk</option
              ><option value="stage">Stage · presentations and screen shares</option></select
            ></label
          ><label class="setting-toggle"
            ><span><strong>Private room</strong></span><input type="checkbox" bind:checked={createPrivate} /></label
          >{/if}{#if formError}<p class="error" role="alert">{formError}</p>{/if}<button
          class="button primary full"
          >{joinCode ? 'Join space' : modal === 'space' ? 'Create space' : 'Create room'}</button
        >
      </form></Modal
    >{/if}
  {#if roomSettings && roomId}<RoomSettings
      room={roomId}
      close={() => (roomSettings = false)}
      removed={() => {
        roomId = '';
      }}
    />{/if}
  {#if modal === 'status'}<Modal title="Set status" close={() => (modal = null)}
      ><label class="field"
        >Status<input bind:value={customStatus} maxlength="100" placeholder="In a meeting until 3" /></label
      ><label class="field"
        >Presence<select bind:value={presenceStatus}
          ><option value="online">Online</option><option value="idle">Away</option><option value="dnd"
            >Do not disturb</option
          ><option value="offline">Invisible</option></select
        ></label
      ><button
        class="button primary full"
        onclick={async () => {
          await setStatus({ status: presenceStatus, text: customStatus });
          modal = null;
          notice('Status updated.');
        }}>Save status</button
      ></Modal
    >{/if}
  {#if toast}<div class="toast" role="status">
      {toast}<button class="icon-button" aria-label="Dismiss notification" onclick={() => (toast = '')}
        ><Icon name="close" size={14} /></button
      >
    </div>{/if}
</div>
