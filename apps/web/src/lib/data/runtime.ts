// Mock-only bootstrap. Production replaces this file with RxFerrix context and a real CallController.
import { useMutation, backend, startDemoActivity, setupMock } from './hooks.svelte';
import { api } from './api';
import { MockCallController } from '../call/controller.svelte';
export function initializeDemo() { setupMock(); }
export function createDemoCall() { const join = useMutation(api.voice.join); return new MockCallController(room => join({ room }), (room, state) => backend.updateCall(room, state)); }
export { startDemoActivity };
