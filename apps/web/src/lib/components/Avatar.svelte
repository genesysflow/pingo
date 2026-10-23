<script lang="ts">
  import { initials } from '../logic/chat';
  let {
    name,
    color = '#b7a2d7',
    size = 36,
    status,
    speaking = false,
    url,
  }: {
    name: string;
    color?: string;
    size?: number;
    status?: string;
    speaking?: boolean;
    url?: string;
  } = $props();
</script>

<span
  class="avatar"
  class:speaking
  style:--avatar-color={color}
  style:--size={`${size}px`}
  title={name}
  aria-label={`${name}${status ? `, ${status}` : ''}`}
>
  {#if url}<img
      src={url}
      alt=""
      style="width:100%;height:100%;object-fit:cover;border-radius:inherit"
    />{:else}<span>{initials(name)}</span>{/if}
  {#if status}<i class="presence {status}"></i>{/if}
</span>
