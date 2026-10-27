import { beforeEach, afterEach, describe, expect, test, vi } from 'vitest';
const bridge = vi.hoisted(() => ({
  invoke: vi.fn(async (_command: string, _args: unknown) => {}),
  callbacks: new Map<string, (event: { payload: unknown }) => void>(),
  stop: vi.fn(),
}));
vi.mock('@tauri-apps/api/core', () => ({ invoke: bridge.invoke }));
vi.mock('@tauri-apps/api/event', () => ({ listen: vi.fn(async (name: string, cb: (e: { payload: unknown }) => void) => {
  bridge.callbacks.set(name, cb); return bridge.stop;
}) }));
import type { Admission } from '../src/lib/call/media';
import { TauriCallController } from '../src/lib/call/tauri.svelte';
const admission = vi.fn(async (): Promise<Admission> => ({ token: `fresh-${admission.mock.calls.length}`, url: '/rtc' }));
let call: TauriCallController;
async function flush() { for (let i = 0; i < 30; i++) await Promise.resolve(); }
function joinArgs() { return bridge.invoke.mock.calls.filter(([name]) => name === 'call_join').at(-1)![1] as { sessionId: string; token: string }; }
function event(type: string, extra: object = {}, sessionId = joinArgs().sessionId) {
  bridge.callbacks.get('native-call')!({ payload: { sessionId, event: { type, ...extra } } });
}
beforeEach(() => {
  vi.useFakeTimers(); vi.clearAllMocks(); bridge.callbacks.clear();
  vi.stubGlobal('localStorage', { getItem: () => null, setItem: vi.fn() });
  call = new TauriCallController(admission, async () => ({ audioProfile: 'voice' }), () => 'user-self');
});
afterEach(() => { call.destroy(); vi.useRealTimers(); vi.unstubAllGlobals(); });
describe('native call controller', () => {
  test('waits for native connection and maps participant IDs into UI user IDs', async () => {
    await call.join('room', 'Friends');
    expect(call.connection).toBe('connecting');
    event('joined', { participantId: 'self', participants: [{ participantId: 'peer', user: 'maya' }] });
    event('connected'); event('data', { data: { type: 'speaking', participantId: 'peer', level: 0.7 } });
    expect(call.connection).toBe('connected'); expect(call.levels.maya).toBe(0.7);
    event('quality', { quality: 'Poor' }); expect(call.quality).toBe('Poor');
    event('playout', { receivedPackets: 12 }); expect(call.receivedPackets).toBe(12);
  });
  test('remote tracks disappear when their participant leaves or the call reconnects', async () => {
    await call.join('room', 'Friends');
    event('joined', { participantId: 'self', participants: [{ participantId: 'peer', user: 'maya' }] });
    event('trackPublished', { track: { participantId: 'peer', mid: 'screen', kind: 'screen' } });
    expect(call.media).toMatchObject([{ mid: 'screen', user: 'maya', native: true }]);
    event('participantLeft', { participantId: 'peer' });
    expect(call.media).toEqual([]);
    event('participantJoined', { participant: { participantId: 'peer', user: 'maya' } });
    event('trackPublished', { track: { participantId: 'peer', mid: 'camera', kind: 'camera' } });
    event('disconnected');
    expect(call.media).toEqual([]); expect(call.participants).toEqual([]);
  });
  test('reconnect obtains fresh admission and ignores old session events', async () => {
    await call.join('room', 'Friends'); const old = joinArgs(); event('connected'); event('disconnected');
    expect(call.connection).toBe('reconnecting');
    await vi.advanceTimersByTimeAsync(500); await flush();
    expect(admission).toHaveBeenCalledTimes(2); expect(joinArgs().token).not.toBe(old.token);
    expect(joinArgs().sessionId).not.toBe(old.sessionId);
    event('connected', {}, old.sessionId); expect(call.connection).toBe('reconnecting');
    event('connected'); expect(call.connection).toBe('connected');
  });
  test('passes TURN credentials and refreshes admission while retaining mute settings', async () => {
    const iceServers = [{ urls: ['turns:relay.test:443?transport=tcp'], username: 'temporary', credential: 'temporary-password' }];
    admission.mockResolvedValueOnce({ token: 'initial-turn', url: '/rtc', iceServers, iceExpiresAt: Date.now() + 301_000 });
    await call.join('room', 'Friends');
    expect(joinArgs()).toMatchObject({ iceServers });
    event('connected'); call.toggleMute();
    await vi.advanceTimersByTimeAsync(1500); await flush();
    expect(admission).toHaveBeenCalledTimes(2);
    expect(joinArgs()).toMatchObject({ settings: { muted: true } });
    event('connected');
    expect(call.connection).toBe('connected');
  });
  test('cancelled admission cannot join after leave', async () => {
    let release!: (value: { token: string; url: string }) => void;
    admission.mockImplementationOnce(() => new Promise(resolve => { release = resolve; }));
    const pending = call.join('room', 'Friends'); await flush(); call.leave();
    release({ token: 'obsolete', url: '/rtc' }); await pending;
    expect(bridge.invoke.mock.calls.some(([name]) => name === 'call_join')).toBe(false);
    expect(call.connection).toBe('idle');
  });
  test('sends native mute/deafen, processing settings and participant-based spatial mixes', async () => {
    await call.join('room', 'Friends');
    event('joined', { participantId: 'self', participants: [{ participantId: 'peer', user: 'maya' }] });
    call.configureAudio({ ...call.audio, echoCancel: false, pushToTalk: true, suppression: 'Strong' });
    call.toggleMute(); call.toggleDeafen(); call.setParticipantVolume('maya', 1.5); call.setSpatialAudio('maya', { volume: 0.5, pan: -0.7 });
    await flush();
    const args = bridge.invoke.mock.calls.filter(([name]) => name === 'call_configure').at(-1)![1];
    expect(args).toMatchObject({ settings: { muted: true, deafened: true, echoCancel: false, pushToTalk: true, suppression: 'Strong', volumes: { peer: { volume: 0.75, pan: -0.7 } } } });
  });
  test('leave cancels reconnect, clears participants and unsubscribes on destroy', async () => {
    await call.join('room', 'Friends'); event('disconnected'); call.leave();
    await vi.advanceTimersByTimeAsync(30000); expect(admission).toHaveBeenCalledTimes(1);
    expect(call.participants).toEqual([]); expect(call.room).toBeNull();
    call.destroy(); await flush(); expect(bridge.stop).toHaveBeenCalledTimes(2);
  });
});
