import { setupRxFerrix, getRxFerrixClient } from '@rxferrix/svelte';
import { backend, startDemoActivity, setupMock } from './hooks.svelte';
import { useMutation, useAuth, api, isDemo } from './index';
import { MockCallController } from '../call/controller.svelte';
import { BrowserCallController } from '../call/browser.svelte';
import { nativeDesktop, serverOrigin } from '../desktop/bridge';
import { TauriCallController } from '../call/tauri.svelte';
export function initializeData() {
  if (isDemo) setupMock();
  else setupRxFerrix(serverOrigin());
}
export function createCall() {
  const join = useMutation(api.voice.join);
  const auth = useAuth();
  if (isDemo)
    return new MockCallController(
      (room) => join({ room }),
      (room, state) => backend.updateCall(room, state),
    );
  const client = getRxFerrixClient();
  const Controller = nativeDesktop ? TauriCallController : BrowserCallController;
  return new Controller(
    (room) => join({ room }),
    (room) => client.query(api.rooms.get, { room }),
    () => auth.userId ?? '',
  );
}
export function startActivity() {
  return isDemo ? startDemoActivity() : () => {};
}
