import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import type { VersionInfo } from '../proto';

/** Mirrors `UpdateStatus` in `apps/desktop/src-tauri/src/updates.rs`. */
export type UpdateStatus =
  | { state: 'disabled' | 'idle' | 'checking' | 'upToDate' }
  | { state: 'downloading'; version: string }
  | { state: 'ready'; version: string; notes: string | null }
  | { state: 'failed'; message: string };

/** The native updater checks and downloads on its own; this mirrors its status for the UI. */
class DesktopUpdates {
  status = $state<UpdateStatus>({ state: 'idle' });
  appVersion = $state('');
  #started = false;
  start() {
    if (this.#started) return;
    this.#started = true;
    void listen<UpdateStatus>('desktop-update', (e) => (this.status = e.payload));
    void invoke<UpdateStatus>('desktop_update_status').then((s) => (this.status = s));
    void invoke<VersionInfo>('desktop_app_info').then((i) => (this.appVersion = i.version));
  }
  async check() {
    this.status = await invoke<UpdateStatus>('desktop_update_check');
  }
  /** Installs the downloaded update and restarts. Refused during a call. */
  install() {
    return invoke('desktop_update_install');
  }
}
export const updates = new DesktopUpdates();
