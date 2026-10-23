import { describe, test, expect } from 'vitest';
import {
  configureSdp,
  qualityLabel,
  ReconnectPolicy,
  microphoneConstraints,
  screenConstraints,
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
