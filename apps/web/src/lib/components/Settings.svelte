<script lang="ts">
  import { ACCENTS } from '../theme';
  import { errorMessage } from '../data/errors';
  import { untrack } from 'svelte';
  import { api, useQuery, useMutation, useAuth, uploadFile, type PresenceStatus } from '../data';
  import type { CallController } from '../call/controller.svelte';
  import Icon from './Icon.svelte';
  import Avatar from './Avatar.svelte';
  import { nativeDesktop, serverOrigin } from '../desktop/bridge';
  import DesktopSettings from '../desktop/DesktopSettings.svelte';
  import NativeAudioControls from '../desktop/NativeAudioControls.svelte';
  let {
    page = 'Appearance',
    theme,
    density,
    space,
    call,
    setTheme,
    setDensity,
    back,
    sound,
    notice,
  }: {
    page?: string;
    theme: string;
    density: string;
    space: string;
    call: CallController;
    setTheme: (v: string) => void;
    setDensity: (v: string) => void;
    back: () => void;
    sound: () => void;
    notice: (text: string) => void;
  } = $props();
  const me = useQuery(api.profiles.me),
    info = useQuery(api.spaces.get, () => ({ space }));
  const updateProfile = useMutation(api.profiles.update),
    status = useMutation(api.presence.setStatus),
    updateSpace = useMutation(api.spaces.update),
    createInvite = useMutation(api.invites.create),
    auth = useAuth(),
    generateUpload = useMutation(api.files.generateUploadUrl),
    leaveSpace = useMutation(api.spaces.leave);
  let profileName = $state(''),
    statusText = $state(''),
    profileColor = $state('#b9a2e7'),
    spaceName = $state(''),
    accent = $state('#8064cc'),
    invite = $state(''),
    tab = $state(untrack(() => page)),
    error = $state(''),
    leaving = $state(false),
    uploading = $state(false);
  let desktop = $state(localStorage.getItem('pingo-notify-desktop') !== 'false'),
    sounds = $state(localStorage.getItem('pingo-notify-sounds') === 'true'),
    mentions = $state(localStorage.getItem('pingo-notify-mentions') !== 'false');
  $effect(() => {
    if (me.data) {
      profileName = me.data.name;
      profileColor = me.data.color;
      statusText = me.data.statusText;
    }
  });
  $effect(() => {
    if (info.data) {
      spaceName = info.data.name;
      accent = info.data.accent;
    }
  });
  async function save() {
    error = '';
    try {
      if (tab === 'Profile') {
        await updateProfile({ name: profileName, color: profileColor });
        await status({ status: me.data?.status ?? 'online', text: statusText });
      } else if (tab === 'Space') await updateSpace({ space, name: spaceName, accent });
      notice('Saved.');
    } catch (e) {
      error = errorMessage(e);
    }
  }
  async function imageUpload(event: Event, target: 'avatar' | 'icon') {
    const file = (event.currentTarget as HTMLInputElement).files?.[0];
    if (!file) return;
    error = '';
    uploading = true;
    try {
      const id = await uploadFile(await generateUpload(), file);
      if (target === 'avatar') await updateProfile({ avatar: id });
      else await updateSpace({ space, icon: id });
      notice('Image saved.');
    } catch (e) {
      error = errorMessage(e);
    } finally {
      uploading = false;
    }
  }
  async function notificationChange() {
    localStorage.setItem('pingo-notify-desktop', String(desktop));
    localStorage.setItem('pingo-notify-sounds', String(sounds));
    localStorage.setItem('pingo-notify-mentions', String(mentions));
    if (desktop && !nativeDesktop && 'Notification' in window && Notification.permission === 'default') {
      const permission = await Notification.requestPermission();
      if (permission !== 'granted')
        notice('Notifications are saved as a preference. Allow them in your browser to receive alerts.');
    }
  }
  const shortcuts = [
    ['Find your way', '⌘ / Ctrl K'],
    ['Shortcut sheet', '?'],
    ['Send a message', 'Enter'],
    ['A new line', 'Shift Enter'],
    ['Bold / italic', '⌘ / Ctrl B / I'],
    ['Close a panel', 'Esc'],
    ['Browse Catch up', 'J / K'],
    ['Mark inbox item done', 'E'],
    ['Move in the lounge', 'Arrow keys'],
  ];
</script>

<div class="settings-view">
  <div class="settings-header">
    <button class="text-button" onclick={back}><Icon name="back" size={14} />Back</button>
  </div>
  <div class="settings-layout">
    <nav class="settings-nav" aria-label="Settings sections">
      {#each [{ title: 'Profile', icon: 'users' as const }, { title: 'Audio & video', icon: 'headphones' as const }, { title: 'Appearance', icon: 'sun' as const }, { title: 'Notifications', icon: 'bell' as const }, { title: 'Keyboard shortcuts', icon: 'keyboard' as const }, { title: 'Space', icon: 'compass' as const }] as t}<button
          class:active={tab === t.title}
          onclick={() => (tab = t.title)}><Icon name={t.icon} size={17} />{t.title}</button
        >{/each}<button class="sign-out" onclick={() => auth.signOut()}
        ><Icon name="logout" size={17} />Sign out</button
      >
    </nav>
    <div class="settings-content">
      <h2>{tab}</h2>
      {#if nativeDesktop && tab === 'Keyboard shortcuts'}<DesktopSettings />{/if}
      {#if tab === 'Appearance'}<h3>Theme</h3>
        <div class="theme-options">
          {#each [{ id: 'light', name: 'Light', icon: 'sun' as const }, { id: 'dark', name: 'Dark', icon: 'moon' as const }, { id: 'contrast', name: 'High contrast', icon: 'contrast' as const }] as t}<button
              class:chosen={theme === t.id}
              onclick={() => setTheme(t.id)}
              aria-pressed={theme === t.id}
              ><div class="theme-preview {t.id}"><i></i><span><b></b><b></b><b></b></span><em></em></div>
              <span
                ><Icon name={t.icon} size={15} />{t.name}{#if theme === t.id}<Icon
                    name="check"
                    size={15}
                  />{/if}</span
              ></button
            >{/each}
        </div>
        <h3>Density</h3>
        <div class="density-options">
          {#each ['comfortable', 'compact'] as d}<button
              class:chosen={density === d}
              aria-pressed={density === d}
              onclick={() => setDensity(d)}
              ><Icon name={d === 'comfortable' ? 'expand' : 'collapse'} size={20} /><span
                ><strong>{d === 'comfortable' ? 'Comfortable' : 'Compact'}</strong><small
                  >{d === 'comfortable' ? 'Larger rows and avatars' : 'More messages on screen'}</small
                ></span
              ><span class="radio-circle">{density === d ? '●' : ''}</span></button
            >{/each}
        </div>
        <h3>Accent for {info.data?.name}</h3>
        <div class="color-swatches">
          {#each ACCENTS as c}<button
              style:--swatch={c}
              class:chosen={info.data?.accent === c}
              aria-label={`Set accent ${c}`}
              aria-pressed={info.data?.accent === c}
              onclick={() => updateSpace({ space, accent: c }).catch((e) => (error = errorMessage(e)))}
              >{#if info.data?.accent === c}<Icon name="check" size={18} />{/if}</button
            >{/each}
        </div>
        <p class="settings-description">Animations follow your system’s reduced-motion setting.</p>
      {:else if tab === 'Profile'}<label class="field"
          >Profile photo<input
            type="file"
            accept="image/*"
            disabled={uploading}
            onchange={(e) => imageUpload(e, 'avatar')}
          /></label
        >
        {#if me.data}<div class="profile-preview">
            <Avatar
              url={me.data?.avatarUrl}
              name={profileName || 'You'}
              color={profileColor}
              size={70}
              status={me.data.status}
            /><span><strong>{profileName}</strong><small>{statusText}</small></span><input
              type="color"
              aria-label="Avatar color"
              bind:value={profileColor}
            />
          </div>{/if}<label class="field">Your name<input bind:value={profileName} maxlength="32" /></label
        ><label class="field"
          >Status<input bind:value={statusText} maxlength="100" placeholder="In a meeting until 3" /></label
        ><label class="field"
          >Presence<select
            value={me.data?.status}
            onchange={(e) =>
              status({ status: e.currentTarget.value as PresenceStatus }).catch(
                (e) => (error = errorMessage(e)),
              )}
            ><option value="online">Online</option><option value="idle">Away</option><option value="dnd"
              >Do not disturb</option
            ><option value="offline">Invisible</option></select
          ></label
        ><button class="button primary" onclick={save}
          >Save profile</button
        >
      {:else if tab === 'Audio & video'}{#if nativeDesktop}<NativeAudioControls {call} />{/if}<div class="audio-summary">
          <dl>
            <div><dt>Mode</dt><dd>{call.audio.studio ? 'Studio · stereo, unprocessed' : 'Voice'}</dd></div>
            <div>
              <dt>Noise suppression</dt><dd
                >{call.audio.studio
                  ? 'Off in Studio'
                  : call.audio.suppression === 'Off'
                    ? 'Off'
                    : call.noiseEngine === 'Browser'
                      ? `${call.audio.suppression} · browser filter (DeepFilterNet unavailable)`
                      : `${call.audio.suppression} · DeepFilterNet3`}</dd
              >
            </div>
          </dl>
          <button class="button" onclick={sound}>Sound check</button>
        </div>
        <label class="setting-toggle"
          ><span
            ><strong>Studio</strong><small
              >Stereo at 256 kbps for music and instruments. Turns off echo cancellation, noise suppression and
              automatic gain. Use headphones.</small
            ></span
          ><input
            type="checkbox"
            role="switch"
            checked={call.audio.studio}
            onchange={(e) => call.configureAudio({ ...call.audio, studio: e.currentTarget.checked })}
          /></label
        ><label class="field"
          >Noise suppression
          <div class="segmented large">
            {#each ['Off', 'Gentle', 'Strong'] as mode}<button
                disabled={call.audio.studio}
                class:active={call.audio.suppression === mode}
                onclick={() =>
                  call.configureAudio({ ...call.audio, suppression: mode as 'Off' | 'Gentle' | 'Strong' })}
                >{mode}</button
              >{/each}
          </div></label
        >
        <p class="settings-description">
          Gentle limits suppression to 18 dB so the room keeps some air. Strong removes as much noise as
          possible.
        </p>
      {:else if tab === 'Notifications'}        {#each [{ label: 'Desktop notifications', detail: 'Show a system notification for new activity.', key: 'desktop' }, { label: 'Only mentions and DMs', detail: 'Mentions, thread replies and direct messages.', key: 'mentions' }, { label: 'Sound', detail: 'Play a short sound when something arrives.', key: 'sounds' }] as setting}<label
            class="setting-toggle"
            ><span><strong>{setting.label}</strong><small>{setting.detail}</small></span><input
              type="checkbox"
              role="switch"
              checked={setting.key === 'desktop' ? desktop : setting.key === 'mentions' ? mentions : sounds}
              onchange={(e) => {
                if (setting.key === 'desktop') desktop = e.currentTarget.checked;
                else if (setting.key === 'mentions') mentions = e.currentTarget.checked;
                else sounds = e.currentTarget.checked;
                void notificationChange();
              }}
            /></label
          >{/each}
        <p class="settings-description">Saved on this device.</p>
      {:else if tab === 'Keyboard shortcuts'}<div class="shortcut-list">
          {#each shortcuts as [label, keys]}<div><span>{label}</span><kbd>{keys}</kbd></div>{/each}
        </div>
      {:else if tab === 'Space'}<label class="field"
          >Space icon<input
            type="file"
            accept="image/*"
            disabled={uploading}
            onchange={(e) => imageUpload(e, 'icon')}
          /></label
        >
        <label class="field">Space name<input bind:value={spaceName} maxlength="32" /></label><label
          class="field color-field"
          >Accent<input type="color" bind:value={accent} /><span>{accent}</span></label
        ><button class="button primary" onclick={save}>Save space</button>
        {#if leaving}<p>Leave this space?</p>
          <button
            class="button danger"
            onclick={async () => {
              try {
                await leaveSpace({ space });
                back();
              } catch (e) {
                error = errorMessage(e);
              }
            }}>Leave space</button
          ><button class="text-button" onclick={() => (leaving = false)}>Stay here</button>{:else}<button
            class="text-button"
            onclick={() => (leaving = true)}>Leave this space</button
          >{/if}
        <h3>Invite people</h3>
        <p class="settings-description">Anyone with the link can join this space for the next 48 hours.</p>
        <button
          class="button"
          onclick={async () => {
            try {
              const code = await createInvite({ space, expiresInHours: 48 });
              invite = `${serverOrigin()}/invite/${code}`;
            } catch (e) {
              error = errorMessage(e);
            }
          }}><Icon name="link" size={14} />Create an invite</button
        >{#if invite}<div class="invite-copy">
            <input aria-label="Invite link" readonly value={invite} /><button
              class="button small"
              onclick={async () => {
                try {
                  await navigator.clipboard.writeText(invite);
                  notice('Invite link copied.');
                } catch {
                  notice('Select and copy the invite link above.');
                }
              }}>Copy</button
            ><a href={invite} class="button small">Preview</a>
          </div>{/if}
        <h3>Roles in this space</h3>
        {#each info.data?.roles ?? [] as role}<details class="role-row">
            <summary
              ><span style:color={role.color}>●</span><strong>{role.name}</strong><span>View permissions</span
              ></summary
            >
            <p class="settings-description">
              {role.name === 'Maker'
                ? 'Can manage this space, messages, rooms, and voice.'
                : 'Can view, send messages, invite, speak, use video, and share a screen.'}
            </p>
          </details>{/each}
        <p class="muted tiny">
          Role assignment will connect when role mutations are available in the backend contract.
        </p>{/if}
      {#if error}<p class="error" role="alert">{error}</p>{/if}
    </div>
  </div>
</div>
