import { invoke, isTauri } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';

export const nativeDesktop = isTauri();
export interface DesktopPreferences {
  serverUrl: string | null;
  pushToTalkShortcut: string | null;
}
let preferences: DesktopPreferences = { serverUrl: null, pushToTalkShortcut: null };
export function serverOrigin() {
  return nativeDesktop ? preferences.serverUrl! : location.origin;
}
export async function loadDesktopPreferences() {
  preferences = await invoke<DesktopPreferences>('desktop_config');
  return preferences;
}
export async function saveDesktopPreferences(next: DesktopPreferences) {
  preferences = await invoke<DesktopPreferences>('desktop_save_config', { preferences: next });
  return preferences;
}
export async function watchInvites() {
  const drain = async () => {
    const paths = await invoke<string[]>('desktop_invites');
    if (!paths.length) return;
    history.pushState({}, '', paths.at(-1)!);
    window.dispatchEvent(new PopStateEvent('popstate'));
  };
  const stop = await listen('desktop-invite', () => void drain());
  await drain();
  return stop;
}
