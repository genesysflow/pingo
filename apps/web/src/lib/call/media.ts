import type { IceServer } from '../proto';

export interface Admission {
  token: string;
  url: string;
  iceServers?: IceServer[];
  iceExpiresAt?: number | null;
}

/** Renew before REST credentials expire; zero/absent expiry keeps direct-only calls unchanged. */
export function iceRenewalDelay(expiresAt: number | null | undefined, now = Date.now()): number | null {
  return expiresAt == null ? null : Math.max(1000, expiresAt - now - 300_000);
}

import type { AudioProfile } from '../data/types';

/** Keep Chromium's audio-level extension and codec list; change only Opus fmtp. */
export function configureSdp(sdp: string, profile: AudioProfile): string {
  return sdp
    .split(/(?=m=)/)
    .map((section) => {
      if (!section.startsWith('m=audio')) return section;
      const opus = section.match(/^a=rtpmap:(\d+) opus\/48000(?:\/2)?\r?$/m)?.[1];
      if (!opus) return section;
      const pattern = new RegExp(`^a=fmtp:${opus} (.*)\\r?$`, 'm');
      const params = new Map(
        (section.match(pattern)?.[1] ?? '')
          .trim()
          .split(';')
          .filter(Boolean)
          .map((p) => {
            const [key, ...value] = p.trim().split('=');
            return [key, value.join('=')];
          }),
      );
      params.set('useinbandfec', '1');
      params.set('usedtx', '1');
      params.set('stereo', profile === 'studio' ? '1' : '0');
      params.set('sprop-stereo', profile === 'studio' ? '1' : '0');
      params.set('maxaveragebitrate', profile === 'studio' ? '256000' : '64000');
      const fmtp = `a=fmtp:${opus} ${[...params].map(([k, v]) => `${k}=${v}`).join(';')}`;
      return pattern.test(section)
        ? section.replace(pattern, fmtp)
        : section.replace(/\r?\n$/, `\r\n${fmtp}\r\n`);
    })
    .join('');
}
export interface NetworkSample {
  received: number;
  lost: number;
  jitter: number;
  rtt: number;
}
export function qualityLabel(sample: NetworkSample): 'Great' | 'OK' | 'Poor' {
  const total = Math.max(0, sample.received) + Math.max(0, sample.lost);
  const loss = total ? Math.max(0, sample.lost) / total : 0;
  if (loss > 0.08 || sample.jitter > 0.08 || sample.rtt > 0.4) return 'Poor';
  if (loss > 0.02 || sample.jitter > 0.03 || sample.rtt > 0.18) return 'OK';
  return 'Great';
}
/** A bounded retry budget is reset only after a healthy connection, never merely a socket open. */
export class ReconnectPolicy {
  attempts = 0;
  nextDelay(): number | null {
    return this.attempts >= 5 ? null : Math.min(8000, 500 * 2 ** this.attempts++);
  }
  connected() {
    this.attempts = 0;
  }
}
export function microphoneConstraints(
  studio: boolean,
  microphone = 'default',
  suppression = true,
): MediaTrackConstraints {
  return {
    deviceId: microphone === 'default' ? undefined : { exact: microphone },
    channelCount: studio ? 2 : 1,
    echoCancellation: !studio,
    noiseSuppression: !studio && suppression,
    autoGainControl: !studio,
  };
}
export function screenConstraints(mode: 'Text' | 'Motion'): MediaTrackConstraints {
  return mode === 'Text'
    ? { width: { ideal: 3840 }, height: { ideal: 2160 }, frameRate: { ideal: 10, max: 15 } }
    : { width: { ideal: 1920 }, height: { ideal: 1080 }, frameRate: { ideal: 60, max: 60 } };
}

export function cameraConstraints(): MediaTrackConstraints {
  return {
    width: { ideal: 1920 },
    height: { ideal: 1080 },
    frameRate: { ideal: 30, max: 30 },
  };
}

export function videoEncodings(kind: 'screen' | 'camera'): RTCRtpEncodingParameters[] {
  return kind === 'screen'
    ? [
        { rid: 'q', scaleResolutionDownBy: 2, maxBitrate: 3_000_000 },
        { rid: 'f', scaleResolutionDownBy: 1, maxBitrate: 12_000_000 },
      ]
    : [
        { rid: 'q', scaleResolutionDownBy: 4, maxBitrate: 300_000 },
        { rid: 'h', scaleResolutionDownBy: 2, maxBitrate: 1_200_000 },
        { rid: 'f', scaleResolutionDownBy: 1, maxBitrate: 4_000_000 },
      ];
}

export interface VideoPublication {
  mid: string;
  height: number;
  encodings: RTCRtpEncodingParameters[];
}

/** Browsers omit RID dimensions; advertise them so the SFU doesn't guess a 3240p full layer. */
export function configureVideoSdp(sdp: string, publications: VideoPublication[]): string {
  return sdp
    .split(/(?=m=)/)
    .map((section) => {
      if (!section.startsWith('m=video')) return section;
      const mid = section.match(/^a=mid:(\S+)\r?$/m)?.[1];
      const publication = publications.find((p) => p.mid === mid);
      if (!publication || !publication.height) return section;
      return section.replace(/^a=rid:(\S+) send([^\r\n]*)/gm, (line, rid, attributes: string) => {
        const encoding = publication.encodings.find((e) => e.rid === rid);
        if (!encoding) return line;
        const params = attributes
          .trim()
          .split(';')
          .filter((p) => p && !/^max-(height|br)=/.test(p.trim()));
        params.push(`max-height=${Math.ceil(publication.height / (encoding.scaleResolutionDownBy ?? 1))}`);
        if (encoding.maxBitrate) params.push(`max-br=${encoding.maxBitrate}`);
        return `a=rid:${rid} send ${params.join(';')}`;
      });
    })
    .join('');
}
export async function gatheredDescription(pc: RTCPeerConnection): Promise<string> {
  if (pc.iceGatheringState !== 'complete')
    await new Promise<void>((resolve, reject) => {
      const check = () => {
        if (pc.iceGatheringState === 'complete') {
          cleanup();
          resolve();
        }
      };
      const closed = () => {
        if (pc.signalingState === 'closed') {
          cleanup();
          reject(new Error('Call closed during ICE gathering.'));
        }
      };
      const timeout = setTimeout(() => {
        cleanup();
        // An unreachable TURN server must not discard usable direct candidates.
        // Finish before the SFU's ten-second offer/answer deadline; later candidates are omitted.
        if (/^a=candidate:/m.test(pc.localDescription?.sdp ?? '')) resolve();
        else reject(new Error('Could not gather network candidates.'));
      }, 6_000);
      const cleanup = () => {
        clearTimeout(timeout);
        pc.removeEventListener('icegatheringstatechange', check);
        pc.removeEventListener('signalingstatechange', closed);
      };
      pc.addEventListener('icegatheringstatechange', check);
      pc.addEventListener('signalingstatechange', closed);
      check();
    });
  if (!pc.localDescription?.sdp) throw new Error('The browser did not produce a session description.');
  return pc.localDescription.sdp;
}
