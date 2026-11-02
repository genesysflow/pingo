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
/** Set while the user is switching servers, so the next load shows the picker even with a server saved. */
const CHOOSING = 'pingo-choose-server';
export const choosingServer = () => sessionStorage.getItem(CHOOSING) === '1';
export function chooseServer() {
  sessionStorage.setItem(CHOOSING, '1');
  location.href = '/';
}
export function finishChoosingServer() {
  sessionStorage.removeItem(CHOOSING);
}
/** Accepts a bare host; local addresses default to http, everything else to https. */
export function normalizeServer(input: string) {
  const value = input.trim().replace(/\/+$/, '');
  if (/^[a-z][a-z0-9+.-]*:\/\//i.test(value)) return value;
  const local = /^(localhost|127\.|10\.|192\.168\.|172\.(1[6-9]|2\d|3[01])\.|\[::1\])/i.test(value);
  return `${local ? 'http' : 'https'}://${value}`;
}
/** Confirms a Pingo (RxFerrix) server answers before we commit to it. */
export async function checkServer(origin: string) {
  let response: Response;
  try {
    response = await fetch(`${origin}/api/auth/providers`, { signal: AbortSignal.timeout(8000) });
  } catch {
    throw new Error(`Couldn’t reach ${origin}. Check the address and that the server is running.`);
  }
  if (!response.ok) throw new Error(`${origin} answered, but it doesn’t look like a Pingo server.`);
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
