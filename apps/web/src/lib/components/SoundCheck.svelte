<script lang="ts">
  import { onMount, onDestroy, untrack } from 'svelte';
  import type { CallController, AudioSettings } from '../call/controller.svelte';
  import Icon from './Icon.svelte';
  import Modal from './Modal.svelte';
  import { nativeDesktop } from '../desktop/bridge';
  import NativeSoundCheck from '../desktop/NativeSoundCheck.svelte';
  import { Microphone } from '../call/microphone';
  let { call, close, complete }: { call: CallController; close: () => void; complete: () => void } = $props();
  let step = $state(0),
    devices = $state<MediaDeviceInfo[]>([]),
    mic = $state(untrack(() => call.audio.microphone)),
    speaker = $state(untrack(() => call.audio.speaker)),
    suppression = $state<AudioSettings['suppression']>(untrack(() => call.audio.suppression)),
    studio = $state(untrack(() => call.audio.studio)),
    meter = $state(0),
    testing = $state(false),
    recording = $state(false),
    remaining = $state(5),
    recordingUrl = $state(''),
    error = $state('');
  let microphone: Microphone | undefined,
    context: AudioContext | undefined,
    recorder: MediaRecorder | undefined,
    animation = 0,
    timeout: ReturnType<typeof setTimeout> | undefined,
    countdown: ReturnType<typeof setInterval> | undefined,
    player = $state<HTMLAudioElement>()!;
  async function enumerate() {
    try {
      devices = (await navigator.mediaDevices?.enumerateDevices()) ?? [];
    } catch {
      /* Default device remains available. */
    }
  }
  function stopStream() {
    cancelAnimationFrame(animation);
    microphone?.stop();
    microphone = undefined;
    if (context) void context.close();
    context = undefined;
    testing = false;
    meter = 0;
  }
  async function testMic() {
    stopStream();
    error = '';
    try {
      if (!navigator.mediaDevices?.getUserMedia)
        throw new Error('Microphone access needs a supported browser on localhost or HTTPS.');
      // The same chain a call uses, so the meter and the clip sound like what others hear.
      microphone = await Microphone.open({ microphone: mic, suppression, studio });
      await enumerate();
      context = new AudioContext();
      const analyser = context.createAnalyser();
      analyser.fftSize = 256;
      context.createMediaStreamSource(microphone.stream).connect(analyser);
      const data = new Uint8Array(analyser.fftSize);
      testing = true;
      const frame = () => {
        if (!testing) return;
        analyser.getByteTimeDomainData(data);
        meter = Math.min(
          1,
          Math.sqrt(data.reduce((s, v) => s + ((v - 128) / 128) ** 2, 0) / data.length) * 5,
        );
        animation = requestAnimationFrame(frame);
      };
      frame();
    } catch (e) {
      error =
        e instanceof Error && e.name === 'NotAllowedError'
          ? 'Microphone permission was declined. Allow microphone access in your browser settings, then try again.'
          : `Couldn’t open the microphone. ${e instanceof Error ? e.message : ''}`;
    }
  }
  async function record() {
    if (!microphone) await testMic();
    if (!microphone) return;
    if (!window.MediaRecorder) {
      error = 'Recording isn’t supported in this browser. The live meter still works.';
      return;
    }
    if (recordingUrl) URL.revokeObjectURL(recordingUrl);
    const chunks: BlobPart[] = [];
    recorder = new MediaRecorder(microphone.stream);
    recorder.ondataavailable = (e) => {
      if (e.data.size) chunks.push(e.data);
    };
    recorder.onstop = () => {
      if (chunks.length)
        recordingUrl = URL.createObjectURL(new Blob(chunks, { type: recorder?.mimeType ?? 'audio/webm' }));
      recording = false;
      clearInterval(countdown);
    };
    recorder.start();
    recording = true;
    remaining = 5;
    countdown = setInterval(() => (remaining = Math.max(0, remaining - 1)), 1000);
    timeout = setTimeout(() => {
      if (recorder?.state === 'recording') recorder.stop();
    }, 5000);
  }
  async function playback() {
    try {
      if ('setSinkId' in player && speaker !== 'default')
        await (player as HTMLAudioElement & { setSinkId(id: string): Promise<void> }).setSinkId(speaker);
      await player.play();
    } catch {
      error = 'Couldn’t play on that speaker. Try your default output.';
    }
  }
  function finish() {
    call.configureAudio({ microphone: mic, speaker, suppression, studio });
    localStorage.setItem('pingo-sound-checked', 'true');
    complete();
  }
  onMount(() => {
    if (nativeDesktop) return;
    void enumerate();
    navigator.mediaDevices?.addEventListener('devicechange', enumerate);
  });
  onDestroy(() => {
    clearTimeout(timeout);
    clearInterval(countdown);
    if (recorder?.state === 'recording') {
      recorder.onstop = null;
      recorder.stop();
    }
    stopStream();
    if (recordingUrl) URL.revokeObjectURL(recordingUrl);
    navigator.mediaDevices?.removeEventListener('devicechange', enumerate);
  });
</script>

{#if nativeDesktop}<NativeSoundCheck {call} {close} {complete} />{:else}
<Modal title="Sound check" {close}>
  <div class="wizard-progress">
    {#each ['Devices', 'Microphone', 'Processing'] as label, i}<button
        class:active={step === i}
        class:complete={step > i}
        onclick={() => (step = i)}><span>{step > i ? '✓' : i + 1}</span>{label}</button
      >{/each}
  </div>
  {#if step === 0}    <label class="field"
      >Microphone<select
        bind:value={mic}
        onchange={() => {
          if (testing) void testMic();
        }}
        ><option value="default">System default microphone</option
        >{#each devices.filter((d) => d.kind === 'audioinput' && d.deviceId && d.deviceId !== 'default') as d}<option
            value={d.deviceId}>{d.label || `Microphone ${d.deviceId.slice(0, 4)}`}</option
          >{/each}</select
      ></label
    ><label class="field"
      >Speaker<select bind:value={speaker}
        ><option value="default">System default speaker</option
        >{#each devices.filter((d) => d.kind === 'audiooutput' && d.deviceId && d.deviceId !== 'default') as d}<option
            value={d.deviceId}>{d.label || `Speaker ${d.deviceId.slice(0, 4)}`}</option
          >{/each}</select
      ></label
    >
    <div class="tip">
      <Icon name="headphones" size={17} />Headphones stop your speakers from leaking into your microphone.
    </div>
  {:else if step === 1}<h3>Test your microphone</h3>
    <p class="settings-description">
      Say a few words and watch the meter, or record five seconds and play it back. The clip runs through
      your current settings and never leaves this device.
    </p>
    <div
      class="mic-meter"
      role="meter"
      aria-label="Microphone input level"
      aria-valuemin="0"
      aria-valuemax="100"
      aria-valuenow={Math.round(meter * 100)}
    >
      {#each Array(32) as _, i}<span class:lit={i / 32 < meter}></span>{/each}
    </div>
    <div class="inline-actions">
      <button class="button" onclick={testMic}
        ><Icon name="mic" size={16} />{testing ? 'Restart microphone' : 'Enable microphone'}</button
      ><button class="button" disabled={recording} onclick={record}
        ><Icon name={recording ? 'stop' : 'circle'} size={16} />{recording
          ? `Recording… ${remaining}s`
          : 'Record 5 seconds'}</button
      ><button class="button soft" disabled={!recordingUrl || recording} onclick={playback}
        ><Icon name="play" size={16} />Play back</button
      >
    </div>
    <audio bind:this={player} src={recordingUrl}></audio>
    <p class="muted tiny">Your browser will ask before using your microphone.</p>
  {:else}<h3>Processing</h3>
    <label class="field"
      >Noise suppression
      <div class="segmented large">
        {#each ['Off', 'Gentle', 'Strong'] as mode}<button
            disabled={studio}
            class:active={suppression === mode}
            onclick={() => (suppression = mode as AudioSettings['suppression'])}>{mode}</button
          >{/each}
      </div></label
    >
    <p class="settings-description">
      {studio
        ? 'Studio sends your microphone unprocessed, in stereo.'
        : suppression === 'Off'
          ? 'Your microphone as it is. Fine in a quiet room.'
          : suppression === 'Gentle'
            ? 'Removes most noise, capped at 18 dB so the room keeps some air.'
            : 'Removes as much noise as possible. Best for loud rooms.'}
    </p>
    <label class="setting-toggle"
      ><span
        ><strong>Studio</strong><small
          >Stereo for music and instruments. Turns off echo cancellation, noise suppression and automatic
          gain.</small
        ></span
      ><input type="checkbox" role="switch" bind:checked={studio} /></label
    >
{/if}
  {#if error}<p class="error" role="alert">{error}</p>{/if}
  <div class="wizard-footer">
    <button class="text-button" onclick={step ? () => step-- : close}
      >{step ? 'Back' : 'Skip for now'}</button
    ><span>{step + 1} of 3</span><button
      class="button primary"
      disabled={recording}
      onclick={() => (step < 2 ? step++ : finish())}
      >{step < 2 ? 'Next' : 'Done'}</button
    >
  </div>
</Modal>

{/if}
