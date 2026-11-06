import { afterEach, beforeEach, expect, test, vi } from 'vitest';
import { createReadObserver } from '../src/lib/logic/read-messages';

let changed: IntersectionObserverCallback;
let visibility: EventTarget & { hidden: boolean };
let reads: ReturnType<typeof createReadObserver>;
let markRead: ReturnType<typeof vi.fn>;
const node = () => ({}) as HTMLElement;
function intersect(target: HTMLElement, visible = true) {
  changed(
    [
      {
        target,
        isIntersecting: visible,
        intersectionRect: { height: visible ? 40 : 0 },
      } as unknown as IntersectionObserverEntry,
    ],
    {} as IntersectionObserver,
  );
}
beforeEach(() => {
  vi.useFakeTimers();
  visibility = Object.assign(new EventTarget(), { hidden: false });
  vi.stubGlobal('document', visibility);
  vi.stubGlobal(
    'IntersectionObserver',
    class {
      constructor(callback: IntersectionObserverCallback) {
        changed = callback;
      }
      observe() {}
      unobserve() {}
      disconnect() {}
    },
  );
  markRead = vi.fn().mockResolvedValue(undefined);
  reads = createReadObserver(markRead);
});
afterEach(() => {
  reads.destroy();
  vi.useRealTimers();
  vi.unstubAllGlobals();
});

test('batches visible messages and leaves virtualized offscreen rows pending', async () => {
  const [first, second, offscreen] = [node(), node(), node()];
  reads.observe(first, 'first');
  reads.observe(second, 'second');
  reads.observe(offscreen, 'offscreen');
  intersect(first);
  intersect(second);
  intersect(offscreen, false);
  await vi.advanceTimersByTimeAsync(100);
  expect(markRead.mock.calls).toEqual([[['first', 'second']]]);
  intersect(first);
  await vi.advanceTimersByTimeAsync(100);
  expect(markRead).toHaveBeenCalledTimes(1);
});

test('a hidden tab keeps messages pending and acknowledges them when it becomes visible', async () => {
  visibility.hidden = true;
  const message = node();
  reads.observe(message, 'hidden');
  intersect(message);
  await vi.advanceTimersByTimeAsync(1000);
  expect(markRead).not.toHaveBeenCalled();
  visibility.hidden = false;
  visibility.dispatchEvent(new Event('visibilitychange'));
  await vi.advanceTimersByTimeAsync(100);
  expect(markRead).toHaveBeenCalledWith(['hidden']);
});

test('scrolling away before the visibility batch does not acknowledge unseen content', async () => {
  const message = node();
  reads.observe(message, 'fleeting');
  intersect(message);
  intersect(message, false);
  await vi.advanceTimersByTimeAsync(100);
  expect(markRead).not.toHaveBeenCalled();
});

test('failed acknowledgements retry while the message is visible and unmount cancels pending work', async () => {
  markRead.mockRejectedValueOnce(new Error('offline'));
  const message = node();
  reads.observe(message, 'retry');
  intersect(message);
  await vi.advanceTimersByTimeAsync(1100);
  expect(markRead).toHaveBeenCalledTimes(2);
  const other = node();
  reads.observe(other, 'unmounted');
  intersect(other);
  reads.destroy();
  await vi.advanceTimersByTimeAsync(100);
  expect(markRead).toHaveBeenCalledTimes(2);
});

test('large visible sets respect the API batch limit and optimistic messages are excluded', async () => {
  for (let i = 0; i < 105; i++) {
    const message = node();
    reads.observe(message, String(i));
    intersect(message);
  }
  const optimistic = node();
  reads.observe(optimistic, 'optimistic:pending');
  intersect(optimistic);
  await vi.advanceTimersByTimeAsync(200);
  expect(markRead.mock.calls.map(([ids]) => ids.length)).toEqual([100, 5]);
});
