import { describe, test, expect } from 'vitest';
import {
  configureSdp,
  qualityLabel,
  ReconnectPolicy,
  microphoneConstraints,
  screenConstraints,
  cameraConstraints,
  videoEncodings,
  configureVideoSdp,
} from '../src/lib/call/media';
const sdp =
  'v=0\r\nm=audio 9 UDP/TLS/RTP/SAVPF 111\r\na=rtpmap:111 opus/48000/2\r\na=fmtp:111 minptime=10;useinbandfec=0\r\na=extmap:1 urn:ietf:params:rtp-hdrext:ssrc-audio-level\r\nm=video 9 UDP/TLS/RTP/SAVPF 96\r\na=rtpmap:96 VP8/90000\r\n';
describe('browser media negotiation', () => {
  test('Opus voice keeps RFC6464 and video, enables FEC/DTX, and is idempotent', () => {
    const voice = configureSdp(sdp, 'voice');
    expect(voice).toContain('useinbandfec=1;usedtx=1;stereo=0;sprop-stereo=0;maxaveragebitrate=64000');
    expect(voice).toContain('minptime=10');
    expect(voice).toContain('urn:ietf:params:rtp-hdrext:ssrc-audio-level');
    expect(voice.split('m=video')[1]).toBe(sdp.split('m=video')[1]);
    expect(configureSdp(voice, 'voice')).toBe(voice);
  });
  test('Studio negotiates stereo at 256 kbps, including absent fmtp', () => {
    const studio = configureSdp(sdp.replace(/a=fmtp:.*\r\n/, ''), 'studio');
    expect(studio).toContain('stereo=1;sprop-stereo=1;maxaveragebitrate=256000');
    expect(studio.match(/a=fmtp:/g)).toHaveLength(1);
    expect(microphoneConstraints(true, 'usb')).toMatchObject({
      deviceId: { exact: 'usb' },
      channelCount: 2,
      echoCancellation: false,
      noiseSuppression: false,
      autoGainControl: false,
    });
  });
  test('Text favors detail at low FPS; Motion requests 1080p60', () => {
    expect(screenConstraints('Text')).toMatchObject({ height: { ideal: 2160 }, frameRate: { max: 15 } });
    expect(screenConstraints('Motion')).toMatchObject({ height: { ideal: 1080 }, frameRate: { ideal: 60 } });
  });
  test('camera requests 1080p30 with device fallback, and video layers have HD budgets', () => {
    expect(cameraConstraints()).toEqual({
      width: { ideal: 1920 }, height: { ideal: 1080 }, frameRate: { ideal: 30, max: 30 },
    });
    expect(videoEncodings('camera').map(e => e.scaleResolutionDownBy)).toEqual([4, 2, 1]);
    expect(videoEncodings('camera').at(-1)?.maxBitrate).toBe(4_000_000);
    expect(videoEncodings('screen').at(-1)?.maxBitrate).toBe(12_000_000);
  });
  test('RID metadata advertises captured heights, keeps other attributes and ignores receive tracks', () => {
    const offer = sdp + 'a=mid:screen\r\na=rid:q send pt=96;max-height=360;max-br=350000\r\na=rid:f send\r\na=rid:q recv\r\na=simulcast:send q;f\r\n';
    const publications = [{ mid: 'screen', height: 2160, encodings: videoEncodings('screen') }];
    const configured = configureVideoSdp(offer, publications);
    expect(configured).toContain('a=rid:q send pt=96;max-height=1080;max-br=3000000\r\n');
    expect(configured).toContain('a=rid:f send max-height=2160;max-br=12000000\r\n');
    expect(configured).toContain('a=rid:q recv\r\n');
    expect(configured).toContain('a=simulcast:send q;f\r\n');
    expect(configured.split('m=video')[0]).toBe(offer.split('m=video')[0]);
    expect(configureVideoSdp(configured, publications)).toBe(configured);
    expect(configureVideoSdp(offer, [{ ...publications[0], mid: 'other' }])).toBe(offer);
    const motion = configureVideoSdp(configured, [{ ...publications[0], height: 1080 }]);
    expect(motion).toContain('a=rid:q send pt=96;max-height=540;');
    expect(motion).toContain('a=rid:f send max-height=1080;');
  });
});
test('network quality reflects recent packet loss, jitter and RTT', () => {
  expect(qualityLabel({ received: 100, lost: 0, jitter: 0.01, rtt: 0.04 })).toBe('Great');
  expect(qualityLabel({ received: 97, lost: 3, jitter: 0.01, rtt: 0.04 })).toBe('OK');
  expect(qualityLabel({ received: 90, lost: 10, jitter: 0.01, rtt: 0.04 })).toBe('Poor');
  expect(qualityLabel({ received: 0, lost: 0, jitter: 0.09, rtt: 0.04 })).toBe('Poor');
  expect(qualityLabel({ received: 10, lost: -1, jitter: 0.01, rtt: 0.3 })).toBe('OK');
});
test('reconnect has a bounded backoff and a healthy connection resets the budget', () => {
  const retry = new ReconnectPolicy();
  expect(Array.from({ length: 6 }, () => retry.nextDelay())).toEqual([500, 1000, 2000, 4000, 8000, null]);
  retry.connected();
  expect(retry.nextDelay()).toBe(500);
});

test('TURN renewal is early and missing credentials leave direct calls alone', async () => {
  const { iceRenewalDelay } = await import('../src/lib/call/media');
  expect(iceRenewalDelay(null, 1000)).toBeNull();
  expect(iceRenewalDelay(undefined, 1000)).toBeNull();
  expect(iceRenewalDelay(3_601_000, 1000)).toBe(3_300_000);
  expect(iceRenewalDelay(500, 1000)).toBe(1000);
});

test('unreachable TURN preserves gathered candidates and an empty gather fails', async () => {
  const { gatheredDescription } = await import('../src/lib/call/media');
  const { vi } = await import('vitest');
  vi.useFakeTimers();
  try {
    const peer = Object.assign(new EventTarget(), {
      iceGatheringState: 'gathering', signalingState: 'stable',
      localDescription: { sdp: 'v=0\r\na=candidate:1 1 udp 1 192.0.2.10 40000 typ host\r\n' },
    });
    const gathered = gatheredDescription(peer as unknown as RTCPeerConnection);
    await vi.advanceTimersByTimeAsync(7000);
    expect(await gathered).toContain('typ host');
    peer.localDescription.sdp = 'v=0\r\n';
    const failed = expect(gatheredDescription(peer as unknown as RTCPeerConnection)).rejects.toThrow('Could not gather');
    await vi.advanceTimersByTimeAsync(7000);
    await failed;
  } finally { vi.useRealTimers(); }
});
