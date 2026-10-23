// Mock-only bootstrap. Production replaces this file with RxFerrix context and a real CallController.
import { useMutation, backend, startDemoActivity, setupMock } from './hooks.svelte';
import { api } from './mock-api';
import { setupRxFerrix } from '@rxferrix/svelte';
import { isDemo } from './index';
import { MockCallController } from '../call/controller.svelte';
export function initializeDemo() {
  if (isDemo) setupMock();
  else setupRxFerrix(location.origin);
}
export function createDemoCall() {
  const join = useMutation(api.voice.join);
  return new MockCallController(
    (room) => join({ room }),
    (room, state) => backend.updateCall(room, state),
  );
}
export function startActivity() { return isDemo ? startDemoActivity() : () => {}; }
