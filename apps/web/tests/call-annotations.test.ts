import { afterEach, beforeEach, expect, test, vi } from 'vitest';
import { BrowserCallController } from '../src/lib/call/browser.svelte';

let call: BrowserCallController;
beforeEach(() => {
  vi.useFakeTimers();
  vi.stubGlobal('localStorage', { getItem: () => null });
  call = new BrowserCallController(vi.fn(), vi.fn(), () => 'user-self');
  call.userId = 'user-self';
  Reflect.set(call, 'self', 'session-self');
});
afterEach(() => {
  call.destroy();
  vi.useRealTimers();
  vi.unstubAllGlobals();
});

test('local annotation feedback remains visible while the data channel connects', () => {
  const send = vi.fn();
  Reflect.set(call, 'channel', { readyState: 'connecting', send });
  call.send({ type: 'pointer', participantId: 'user-self', shareMid: 'screen', pos: [0.25, 0.75] });
  expect(call.pointers['user-self']).toMatchObject({ pos: { x: 0.25, y: 0.75 }, shareMid: 'screen' });
  call.send({
    type: 'stroke',
    participantId: 'user-self',
    shareMid: 'screen',
    points: [
      [0, 0],
      [1, 1],
    ],
    color: '#ffcc33',
  });
  expect(call.strokes).toMatchObject([
    {
      user: 'user-self',
      shareMid: 'screen',
      points: [
        [0, 0],
        [1, 1],
      ],
    },
  ]);
  expect(send).not.toHaveBeenCalled();
});

test('connected annotations use the session identity on the wire and the user identity for local feedback', () => {
  const send = vi.fn();
  Reflect.set(call, 'channel', { readyState: 'open', send });
  call.send({ type: 'pointer', participantId: 'user-self', shareMid: 'screen', pos: [0.5, 0.5] });
  expect(JSON.parse(send.mock.calls[0][0])).toMatchObject({
    participantId: 'session-self',
    shareMid: 'screen',
    pos: [0.5, 0.5],
  });
  expect(call.pointers['user-self']).toMatchObject({ pos: { x: 0.5, y: 0.5 } });
  call.send({ type: 'pointer', participantId: 'user-self', shareMid: 'screen', pos: null });
  expect(call.pointers['user-self']).toBeUndefined();
});
