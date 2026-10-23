<script lang="ts">
  import type { CallController } from '../call/controller.svelte';
  let { call }: { call: CallController } = $props();
</script>

<label class="setting-toggle"><span><strong>Echo cancellation</strong><small>Remove speaker echo from your microphone.</small></span><input type="checkbox" role="switch" disabled={call.audio.studio} checked={call.audio.echoCancel ?? true} onchange={e => call.configureAudio({ ...call.audio, echoCancel: e.currentTarget.checked })} /></label>
<label class="setting-toggle"><span><strong>Push-to-talk</strong><small>Hold Space in Pingo, or your configured global shortcut.</small></span><input type="checkbox" role="switch" checked={call.audio.pushToTalk ?? false} onchange={e => call.configureAudio({ ...call.audio, pushToTalk: e.currentTarget.checked })} /></label>
