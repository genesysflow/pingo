<script lang="ts">
  import { errorMessage } from './lib/data/errors';
  import Flourish from './lib/components/Flourish.svelte';
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
  import RoomSettings from './lib/components/RoomSettings.svelte';
  import Modal from './lib/components/Modal.svelte';
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
  const spaces = useQuery(api.spaces.list, () => (auth.isAuthenticated ? {} : 'skip')),
    rooms = useQuery(api.rooms.list, () => (auth.isAuthenticated && space ? { space } : 'skip')),
    me = useQuery(api.profiles.me),
    dms = useQuery(api.dms.list, () => (auth.isAuthenticated ? {} : 'skip')),
    inboxCount = useQuery(api.inbox.count, () => (auth.isAuthenticated ? {} : 'skip'));
  const roomInfo = useQuery(api.rooms.get, () =>
    auth.isAuthenticated && roomId ? { room: roomId } : 'skip',
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
    if (!auth.isAuthenticated || !spaces.data) return;
    if (!spaces.data.some((s) => s._id === space)) {
      space = spaces.data[0]?._id ?? '';
      roomId = '';
    }
    if (space && !roomId && rooms.data?.length) navigate(rooms.data[0]._id);
  });
  $effect(() => {
    if (auth.isAuthenticated) void heartbeat().catch((e) => notice(errorMessage(e)));
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
    else notice('Your sound is all set. Good conversations ahead.');
  }
  function action(name: string) {
    if (name === 'call' && currentRoom.data) void join(currentRoom.data);
    if (name === 'share') {
      if (!call.room) {
        if (currentRoom.data) {
          void join(currentRoom.data);
          notice('Join the call, then use Share to try the screen preview.');
        }
      } else {
        if (!call.sharing) call.toggleShare();
        stage = true;
        view = 'chat';
      }
    }
    if (name === 'mute') {
      if (call.room) call.toggleMute();
      else notice('Hop into a call to use your microphone.');
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
      if (auth.isAuthenticated) void heartbeat().catch((e) => notice(errorMessage(e)));
    }, 60_000);
    return () => {
      stop();
      clearInterval(interval);
      clearTimeout(toastTimer);
      call.destroy();
    };
  });
</script>

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
      ? 'Pull up a chair'
      : !auth.isAuthenticated || path === '/signin'
        ? 'Welcome'
        : view === 'inbox'
          ? 'Catch up'
          : view === 'settings'
            ? 'Make yourself at home'
            : `${currentRoom.data?.name ?? 'Your people'}`} · Pingo</title
  ></svelte:head
>
<div
  class="app-root"
  data-theme={theme}
  data-density={density}
  style:--accent={currentSpace?.accent ?? '#8064cc'}
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
  {:else if spaces.isLoading}<main class="empty-state" aria-busy="true">Finding your people…</main>
  {:else if spaces.error}<main class="empty-state">
      <p role="alert">{errorMessage(spaces.error)}</p>
      <button class="button" onclick={() => auth.signOut()}>Sign out</button>
    </main>
  {:else if spaces.data?.length === 0}<main class="empty-state onboarding">
      <img src="/pingo.svg" width="64" alt="Pingo" />
      <h1>Make a little space.</h1>
      <p>Create a space for your people, or paste an invite to join them.</p>
      <form
        onsubmit={(e) => {
          modal = 'space';
          void submitCreate(e);
        }}
      >
        <label class="field"
          >Space name<input bind:value={createName} maxlength="32" required={!joinCode} /></label
        >
        <div class="auth-divider"><span>or come on in</span></div>
        <label class="field">Invite link or code<input bind:value={joinCode} /></label>
        {#if formError}<p class="error" role="alert">{formError}</p>{/if}
        <button class="button primary">{joinCode ? 'Join your people' : 'Create space'}</button>
      </form>
      <button class="text-button" onclick={() => auth.signOut()}>Sign out</button>
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
      ><span class="rail-divider"></span>{#each spaces.data ?? [] as s, i}<button
          class="space-icon"
          class:active={space === s._id}
          style:--space-color={s.accent}
          aria-label={`${s.name}${s.mentions ? `, ${s.mentions} mentions` : ''}`}
          aria-current={space === s._id ? 'page' : undefined}
          title={s.name}
          onclick={() => selectSpace(s._id)}
          >{#if s.iconUrl}<img
              src={s.iconUrl}
              alt=""
              style="width:100%;height:100%;object-fit:cover;border-radius:inherit"
            />{:else if i === 0}<Icon name="sparkles" size={25} />{:else if i === 1}<Icon
              name="moon"
              size={23}
            />{:else}<Icon name="sprout" size={24} />{/if}{#if s.mentions}<span class="rail-badge"
              >{s.mentions}</span
            >{:else if s.unread}<span class="rail-unread"></span>{/if}</button
        >{/each}<button
        class="rail-add"
        aria-label="Create or join a space"
        onclick={() => {
          modal = 'space';
          createName = '';
          joinCode = '';
          formError = '';
        }}><Icon name="plus" size={22} /></button
      >
      <div class="rail-bottom">
        <button class="icon-button" aria-label="Keyboard shortcuts" onclick={() => (shortcutSheet = true)}
          ><Icon name="help" size={21} /></button
        ><button class="self-avatar" aria-label="Your profile" onclick={() => settings('Profile')}
          ><Avatar
            url={me.data?.avatarUrl}
            name={me.data?.name ?? 'Alex Morgan'}
            color={me.data?.color}
            size={36}
            status={me.data?.status}
          /></button
        >
      </div>
    </nav>
    <aside class="sidebar" class:mobile-open={mobileSidebar} aria-label="Rooms and direct messages">
      <div class="space-heading">
        <button
          class="space-heading-main"
          aria-expanded={workspaceMenu}
          onclick={() => (workspaceMenu = !workspaceMenu)}
          ><span class="space-wordmark"
            >{currentSpace?.name ?? 'Your space'}<Icon name="down" size={16} /></span
          ><span>A place to make good things.</span></button
        ><button
          class="icon-button mobile-close"
          aria-label="Close sidebar"
          onclick={() => (mobileSidebar = false)}><Icon name="close" /></button
        >{#if workspaceMenu}<div class="workspace-menu">
            <button onclick={() => settings('Space')}
              ><Icon name="settings" size={16} />Space settings & invites</button
            ><button
              onclick={() => {
                modal = 'space';
                workspaceMenu = false;
              }}><Icon name="plus" size={16} />Create or join a space</button
            ><button onclick={() => settings('Profile')}><Icon name="users" size={16} />Your profile</button>
          </div>{/if}
      </div>
      <button class="quick-find" onclick={() => (palette = true)}
        ><Icon name="search" size={16} /><span>Find your way</span><kbd>⌘ K</kbd></button
      ><button class="catch-up-entry" class:active={view === 'inbox'} onclick={catchUp}
        ><Icon name="inbox" size={18} /><span>Catch up</span>{#if inboxCount.data}<span class="quiet-badge"
            >{inboxCount.data}</span
          >{:else}<Icon name="check" size={14} />{/if}</button
      >
      <div class="sidebar-scroll">
        <div class="section-label">
          <span>YOUR ROOMS</span><button
            aria-label="Create a room"
            onclick={() => {
              modal = 'room';
              createName = '';
              formError = '';
            }}><Icon name="plus" size={15} /></button
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
                ><Icon name={r.kind === 'stage' ? 'radio' : r.private ? 'lock' : 'hash'} size={17} /><span
                  >{r.name}</span
                >{#if r.mentions}<span class="mention-badge">{r.mentions}</span>{:else if r.unread}<span
                    class="unread-dot"
                  ></span>{/if}</button
              >{#if r.voice.length}<div class="room-voice">
                  <span class="voice-branch"></span><button
                    class="voice-people"
                    aria-label={`${r.voice.length} people in ${r.name} call`}
                    onclick={() => join(r)}
                    >{#each r.voice.slice(0, 3) as p}<Avatar
                        name={p.name}
                        color={p.color}
                        size={21}
                        speaking={(call.levels[p.user] ?? 0) > 0.5}
                      />{/each}<span
                      >{r.voice.length === 1 ? r.voice[0].name.split(' ')[0] : 'A few good people'}</span
                    ></button
                  >{#if call.room !== r._id}<button class="hop-in" onclick={() => join(r)}
                      >Hop in<Icon name="upRight" size={11} /></button
                    >{/if}
                </div>{/if}
            </div>{/each}
        </nav>
        <div class="section-label lounge-label">
          <span>A LITTLE ROOM TO HANG</span><Icon name="sprout" size={14} />
        </div>
        {#each (rooms.data ?? []).filter((r) => r.kind === 'lounge') as r}<button
            class="lounge-sidebar-card"
            class:active={roomId === r._id && view === 'chat'}
            onclick={() => navigate(r._id)}
            ><span class="lounge-card-icon"><Icon name="sprout" size={23} /></span><span
              ><strong>{r.name}</strong><small
                >{r.voice.length ? `${r.voice.length} people, no agenda` : 'A quiet little corner'}</small
              ></span
            ><Icon name="right" size={15} /></button
          >{/each}
        <div class="section-label dm-label">
          <span>DIRECT MESSAGES</span><button
            aria-label="New direct message"
            onclick={() => {
              palette = true;
            }}><Icon name="plus" size={15} /></button
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
                size={27}
                status={dm.other.status}
              /><span>{dm.other.name}</span>{#if dm.unread}<span class="unread-dot"></span>{/if}</button
            >{/each}
        </nav>
        <button class="invite-people-button" onclick={() => settings('Space')}
          ><Icon name="plus" size={15} />Good people welcome.</button
        >
      </div>
      <div class="sidebar-bottom">
        <div class="sidebar-note">
          <span><Flourish /></span>
          <p>A good day to make<br />something together.</p>
        </div>
        <div class="sidebar-user">
          <button onclick={() => action('status')}
            ><span class="live-dot"></span><span
              >{me.data?.status === 'dnd'
                ? 'In the flow'
                : me.data?.status === 'idle'
                  ? 'A little away'
                  : me.data?.status === 'offline'
                    ? 'Taking a quiet moment'
                    : 'Here for the good stuff'}</span
            ><Icon name="down" size={12} /></button
          ><button class="icon-button" aria-label="Open settings" onclick={() => settings()}
            ><Icon name="settings" size={17} /></button
          >
        </div>
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
                      ? 'sprout'
                      : currentRoom.data?.kind === 'dm'
                        ? 'mail'
                        : currentRoom.data?.kind === 'stage'
                          ? 'radio'
                          : 'hash'}
              size={23}
            /></span
          >
          <div>
            <h1>
              {view === 'inbox'
                ? 'Catch up'
                : view === 'settings'
                  ? 'Your little preferences'
                  : stage
                    ? call.roomName
                    : (currentRoom.data?.name ?? 'Your room')}
            </h1>
            <p>
              {view === 'inbox'
                ? 'The things for you, all in one place.'
                : view === 'settings'
                  ? 'A place that feels like you.'
                  : stage
                    ? 'Same room. A little closer.'
                    : currentRoom.data?.topic}
            </p>
          </div>
        </div>
        <div class="header-actions">
          {#if view === 'chat'}{#if currentRoom.data && currentRoom.data.myPerms & 8 && currentRoom.data.kind !== 'dm'}<button
                class="icon-button"
                aria-label="Room settings"
                onclick={() => (roomSettings = true)}><Icon name="settings" size={18} /></button
              >{/if}<button class="icon-button" aria-label="Search messages" onclick={() => (palette = true)}
              ><Icon name="search" size={19} /></button
            ><button
              class="icon-button"
              aria-label="Show members"
              aria-pressed={members}
              onclick={() => {
                members = !members;
                thread = null;
              }}><Icon name="users" size={19} /></button
            ><span class="header-divider"></span><button
              class="header-call-button"
              onclick={() => {
                if (call.room) {
                  stage = true;
                } else if (currentRoom.data) void join(currentRoom.data);
              }}
              ><Icon name="headphones" size={17} /><span>{call.room ? 'Your call' : 'Start a call'}</span
              ></button
            >{:else}<button class="icon-button" aria-label="Find your way" onclick={() => (palette = true)}
              ><Icon name="search" size={19} /></button
            ><button class="icon-button" aria-label="Change theme" onclick={() => action('theme')}
              ><Icon name={theme === 'light' ? 'moon' : 'sun'} size={19} /></button
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
            <Icon name="compass" size={40} />
            <h2>A new little corner.</h2>
            <p>Choose a room to get comfortable.</p>
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
        expand={() => {
          stage = true;
          view = 'chat';
        }}
        {notice}
      />{/if}
  {/if}
  {#if palette && auth.isAuthenticated}<Palette
      {space}
      rooms={rooms.data ?? []}
      spaces={spaces.data ?? []}
      close={() => (palette = false)}
      {navigate}
      {action}
    />{/if}
  {#if shortcutSheet}<Modal title="A few little shortcuts." close={() => (shortcutSheet = false)}
      ><p class="settings-description">Find your flow, one key at a time.</p>
      <div class="shortcut-list">
        {#each [['Find your way', 'Ctrl / ⌘ K'], ['This little sheet', '?'], ['Send a message', 'Enter'], ['A new line', 'Shift Enter'], ['Bold / italic', 'Ctrl / ⌘ B / I'], ['Browse Catch up', 'J / K'], ['Mark as done in Catch up', 'E'], ['Move in the lounge', '← ↑ ↓ →'], ['Point / draw on share', 'Enter / D'], ['Close a panel', 'Escape']] as [label, key]}<div
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
      title={modal === 'space' ? 'Make a little space.' : 'A new room for good ideas.'}
      close={() => (modal = null)}
      ><p class="settings-description">
        {modal === 'space'
          ? 'Start something with your people, or find a place you’ve been invited to.'
          : 'Every room has a chat and a place to talk. Pick the feeling.'}
      </p>
      <form onsubmit={submitCreate}>
        <label class="field"
          >{modal === 'space' ? 'Space name' : 'Room name'}<input
            bind:value={createName}
            maxlength="32"
            placeholder={modal === 'space' ? 'Something lovely' : 'the-next-good-thing'}
            required={!joinCode}
          /></label
        >{#if modal === 'space'}<div class="auth-divider"><span>or come on in</span></div>
          <label class="field"
            >Invite link or code<input bind:value={joinCode} placeholder="fieldwork-friends" /></label
          >{:else}<label class="field"
            >Room feeling<select bind:value={roomKind}
              ><option value="chat">Chat · ideas, updates, and a place to talk</option><option value="lounge"
                >Lounge · wander into a conversation</option
              ><option value="stage">Stage · something worth sharing</option></select
            ></label
          ><label class="setting-toggle"
            ><span>Private room</span><input type="checkbox" bind:checked={createPrivate} /></label
          >{/if}{#if formError}<p class="error" role="alert">{formError}</p>{/if}<button
          class="button primary full"
          >{joinCode
            ? 'Join your people'
            : modal === 'space'
              ? 'Make yourself at home'
              : 'Open the door'}<Icon name="arrowRight" size={16} /></button
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
  {#if modal === 'status'}<Modal title="What’s your world like today?" close={() => (modal = null)}
      ><label class="field"
        >A little status<input
          bind:value={customStatus}
          maxlength="100"
          placeholder="In the flow, out for coffee…"
        /></label
      ><label class="field"
        >Your presence<select bind:value={presenceStatus}
          ><option value="online">Here and happy to chat</option><option value="idle">A little away</option
          ><option value="dnd">In the flow · do not disturb</option><option value="offline"
            >Taking a quiet moment</option
          ></select
        ></label
      ><button
        class="button primary full"
        onclick={async () => {
          await setStatus({ status: presenceStatus, text: customStatus });
          modal = null;
          notice('Your people are in the loop.');
        }}>That’s me<Icon name="check" size={16} /></button
      ></Modal
    >{/if}
  {#if toast}<div class="toast" role="status">
      <span class="toast-spark"><Flourish /></span>{toast}<button
        class="icon-button"
        aria-label="Dismiss notification"
        onclick={() => (toast = '')}><Icon name="close" size={15} /></button
      >
    </div>{/if}
</div>
