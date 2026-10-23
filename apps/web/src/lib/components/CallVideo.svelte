<script lang="ts">
  import type { CallController } from '../call/controller.svelte';
  let {
    stream,
    mid = '',
    call,
    label = 'Shared screen',
  }: {
    stream: MediaStream | null;
    mid?: string;
    call: CallController;
    label?: string;
  } = $props();
  function attachCanvas(node: HTMLCanvasElement) {
    const stop = call.attachVideo?.(mid, node);
    const resize = new ResizeObserver(() => call.setVideoQuality(mid, node.getBoundingClientRect().height * devicePixelRatio));
    const visibility = new IntersectionObserver(([entry]) => call.setVideoQuality(mid, entry.isIntersecting ? node.getBoundingClientRect().height * devicePixelRatio : 0));
    resize.observe(node); visibility.observe(node);
    return { destroy() { stop?.(); resize.disconnect(); visibility.disconnect(); } };
  }
  function attach(node: HTMLVideoElement) {
    node.srcObject = stream;
    void node.play().catch(() => {});
    const resize = new ResizeObserver(() => {
      if (mid) call.setVideoQuality(mid, node.getBoundingClientRect().height * devicePixelRatio);
    });
    const visibility = new IntersectionObserver(([entry]) => {
      if (mid)
        call.setVideoQuality(
          mid,
          entry.isIntersecting ? node.getBoundingClientRect().height * devicePixelRatio : 0,
        );
    });
    resize.observe(node);
    visibility.observe(node);
    return {
      destroy() {
        resize.disconnect();
        visibility.disconnect();
        node.srcObject = null;
      },
    };
  }
</script>

<!-- Audio is routed separately through each participant's Web Audio mixer. -->
{#if call.attachVideo && mid}<canvas use:attachCanvas aria-label={label}></canvas>{:else}<video use:attach autoplay muted playsinline aria-label={label}></video>{/if}

<style>
  video, canvas {
    width: 100%;
    height: 100%;
    object-fit: contain;
    display: block;
    background: #17141f;
  }
</style>
