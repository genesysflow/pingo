<script lang="ts">
  import { invoke } from '@tauri-apps/api/core';
  import { api, useQuery, useAuth, type InboxItem } from '../data';
  import { nativeDesktop } from '../desktop/bridge';
  // Alerts for new inbox items (mentions, thread replies, DMs) while Pingo is in the background, following
  // the Notifications settings. The inbox is already the server's filter for what deserves attention.
  const auth = useAuth();
  const inbox = useQuery(api.inbox.list, () => (auth.isAuthenticated ? {} : 'skip'));
  let seen: Set<string> | undefined;
  let audio: AudioContext | undefined;
  function title(item: InboxItem) {
    const who = item.message.author.name;
    return item.kind === 'dm'
      ? `${who} sent a message`
      : item.kind === 'reply'
        ? `${who} replied in a thread`
        : `${who} mentioned you`;
  }
  function chime() {
    try {
      audio ??= new AudioContext();
      const now = audio.currentTime;
      for (const [i, freq] of [880, 1320].entries()) {
        const osc = audio.createOscillator(),
          gain = audio.createGain();
        osc.frequency.value = freq;
        gain.gain.setValueAtTime(0, now + i * 0.09);
        gain.gain.linearRampToValueAtTime(0.08, now + i * 0.09 + 0.01);
        gain.gain.exponentialRampToValueAtTime(0.0001, now + i * 0.09 + 0.25);
        osc.connect(gain).connect(audio.destination);
        osc.start(now + i * 0.09);
        osc.stop(now + i * 0.09 + 0.3);
      }
    } catch {
      /* Autoplay policy can block audio until the first interaction. */
    }
  }
  function alert(item: InboxItem) {
    const body = item.message.body.slice(0, 4096);
    if (nativeDesktop) void invoke('desktop_notify', { title: title(item), body }).catch(console.error);
    else if ('Notification' in window && Notification.permission === 'granted')
      new Notification(title(item), { body, tag: item._id, icon: '/pingo.svg' });
  }
  $effect(() => {
    if (!auth.isAuthenticated) {
      seen = undefined;
      return;
    }
    const items = inbox.data;
    if (!items) return;
    const previous = seen;
    seen = new Set(items.map((i) => i._id));
    if (!previous || (document.hasFocus() && document.visibilityState === 'visible')) return;
    const fresh = items.filter((i) => !previous.has(i._id) && !i.done);
    if (!fresh.length) return;
    if (localStorage.getItem('pingo-notify-desktop') !== 'false') fresh.forEach(alert);
    if (localStorage.getItem('pingo-notify-sounds') === 'true') chime();
  });
</script>
