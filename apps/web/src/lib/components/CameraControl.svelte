<script lang="ts">
  import type { CallController } from '../call/controller.svelte';
  import Icon from './Icon.svelte';

  let { call }: { call: CallController } = $props();
  const id = $props.id();
  let open = $state(false);
  let control: HTMLDivElement;
  let trigger = $state<HTMLButtonElement>();

  function dismissOutside(event: Event) {
    if (open && event.target instanceof Node && !control.contains(event.target)) open = false;
  }

  function escape(event: KeyboardEvent) {
    if (open && event.key === 'Escape') {
      event.stopPropagation();
      open = false;
      trigger?.focus();
    }
  }
</script>

<svelte:window onpointerdown={dismissOutside} onfocusin={dismissOutside} />

<div class="camera-control" bind:this={control}>
  <button
    class:camera-split={call.backgroundBlurSupported}
    class:control-on={call.camera}
    disabled={call.cameraBusy}
    aria-label={call.camera ? 'Turn camera off' : 'Turn camera on'}
    title={call.camera ? 'Turn camera off' : 'Turn camera on'}
    onclick={() => call.toggleCamera()}><Icon name="video" size={18} /></button
  >
  {#if call.backgroundBlurSupported}
    <button
      class="camera-settings"
      class:control-on={call.camera}
      bind:this={trigger}
      aria-label="Camera settings"
      aria-expanded={open}
      aria-controls={`${id}-effects`}
      title="Camera settings"
      onclick={() => (open = !open)}
      onkeydown={escape}><Icon name="down" size={14} /></button
    >
    {#if open}
      <div class="camera-effects" id={`${id}-effects`} role="group" aria-label="Camera effects">
        <span class="effects-heading">Camera effects</span>
        <button
          class="blur-option"
          role="switch"
          aria-checked={call.backgroundBlur}
          aria-label="Blur camera background"
          disabled={call.cameraBusy}
          title={call.cameraBusy ? 'Updating camera…' : 'Blur camera background'}
          onclick={() => call.setBackgroundBlur(!call.backgroundBlur)}
          onkeydown={escape}
        >
          <Icon name="sparkles" size={18} />
          <span>Background blur</span>
          <span class="blur-switch" class:enabled={call.backgroundBlur} aria-hidden="true"></span>
        </button>
      </div>
    {/if}
  {/if}
</div>

<style>
  .camera-control {
    position: relative;
    display: flex;
    gap: 1px;
  }
  .camera-control .camera-split {
    border-radius: 8px 3px 3px 8px;
  }
  .camera-control .camera-settings {
    width: 24px;
    border-radius: 3px 8px 8px 3px;
  }
  .camera-settings[aria-expanded='true'] {
    background: var(--raised);
  }
  .camera-effects {
    position: absolute;
    bottom: calc(100% + 14px);
    left: 50%;
    transform: translateX(-50%);
    z-index: 10;
    width: 248px;
    padding: 8px;
    border: 1px solid var(--border-strong);
    border-radius: 12px;
    background: var(--surface);
    box-shadow: 0 8px 24px rgb(0 0 0 / 0.3);
  }
  .effects-heading {
    display: block;
    padding: 6px 8px 10px;
    color: var(--muted);
    font-size: 12px;
    font-weight: 600;
  }
  .camera-effects .blur-option {
    width: 100%;
    height: auto;
    min-height: 40px;
    justify-content: flex-start;
    gap: 10px;
    padding: 10px 8px;
    font-size: 13px;
  }
  .blur-switch {
    flex-shrink: 0;
    width: 28px;
    height: 16px;
    margin-left: auto;
    padding: 3px;
    border-radius: 10px;
    background: var(--border-strong);
  }
  .blur-switch::after {
    content: '';
    display: block;
    width: 10px;
    height: 10px;
    border-radius: 50%;
    background: var(--text);
  }
  .blur-switch.enabled {
    background: var(--accent);
  }
  .blur-switch.enabled::after {
    transform: translateX(12px);
    background: var(--page);
  }
</style>
