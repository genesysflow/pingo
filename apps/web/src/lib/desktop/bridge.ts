import { invoke, isTauri } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import type { VersionInfo } from '../proto';

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
export interface ServerCheck {
  origin: string;
  server: VersionInfo;
  app: VersionInfo;
  compatibility: 'compatible' | 'clientTooOld' | 'serverTooOld';
}
/** Confirms a Pingo server answers and speaks a protocol this app supports. Rejects with a readable message. */
export const checkServer = (server: string) => invoke<ServerCheck>('desktop_server_check', { server });
/** Why this app can't use the server, or null when it can. */
export function incompatibility(check: ServerCheck) {
  const host = new URL(check.origin).host;
  if (check.compatibility === 'clientTooOld')
    return `${host} runs a newer Pingo (${check.server.version}) that needs an updated app. Update Pingo, then try again.`;
  if (check.compatibility === 'serverTooOld')
    return `${host} runs an older Pingo (${check.server.version}) that this app no longer supports. Ask its admin to update it, or use an older app.`;
  return null;
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
