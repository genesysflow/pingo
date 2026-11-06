<script lang="ts">
  import type { CallController } from '../call/controller.svelte';
  let {
    stream,
    mid = '',
    call,
    label = 'Shared screen',
    onDimensions,
    minHeight = 0,
  }: {
    stream: MediaStream | null;
    mid?: string;
    call: CallController;
    label?: string;
    onDimensions?: (width: number, height: number) => void;
    minHeight?: number;
  } = $props();
  function attachCanvas(node: HTMLCanvasElement) {
    const stop = call.attachVideo?.(mid, node);
    const dimensions = new MutationObserver(() => onDimensions?.(node.width, node.height));
    dimensions.observe(node, { attributes: true, attributeFilter: ['width', 'height'] });
    const resize = new ResizeObserver(() =>
      call.setVideoQuality(mid, Math.max(minHeight, node.getBoundingClientRect().height * devicePixelRatio)),
    );
    const visibility = new IntersectionObserver(([entry]) =>
      call.setVideoQuality(
        mid,
        entry.isIntersecting
          ? Math.max(minHeight, node.getBoundingClientRect().height * devicePixelRatio)
          : 0,
      ),
    );
    resize.observe(node);
    visibility.observe(node);
    return {
      destroy() {
        stop?.();
        dimensions.disconnect();
        resize.disconnect();
        visibility.disconnect();
      },
    };
  }
  function attach(node: HTMLVideoElement) {
    node.srcObject = stream;
    const dimensions = () => {
      if (node.videoWidth && node.videoHeight) onDimensions?.(node.videoWidth, node.videoHeight);
    };
    node.addEventListener('loadedmetadata', dimensions);
    node.addEventListener('resize', dimensions);
    void node.play().catch(() => {});
    const resize = new ResizeObserver(() => {
      if (mid)
        call.setVideoQuality(
          mid,
          Math.max(minHeight, node.getBoundingClientRect().height * devicePixelRatio),
        );
    });
    const visibility = new IntersectionObserver(([entry]) => {
      if (mid)
        call.setVideoQuality(
          mid,
          entry.isIntersecting
            ? Math.max(minHeight, node.getBoundingClientRect().height * devicePixelRatio)
            : 0,
        );
    });
    resize.observe(node);
    visibility.observe(node);
    return {
      destroy() {
        resize.disconnect();
        visibility.disconnect();
        node.removeEventListener('loadedmetadata', dimensions);
        node.removeEventListener('resize', dimensions);
        node.srcObject = null;
      },
    };
  }
</script>

<!-- Audio is routed separately through each participant's Web Audio mixer. -->
{#if call.attachVideo && mid}<canvas use:attachCanvas aria-label={label}></canvas>{:else}<video
    use:attach
    autoplay
    muted
    playsinline
    aria-label={label}
  ></video>{/if}

<style>
  video,
  canvas {
    width: 100%;
    height: 100%;
    object-fit: contain;
    display: block;
    background: #0b0b0c;
  }
</style>
