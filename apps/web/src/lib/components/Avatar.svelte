<script lang="ts">
  import { initials } from '../logic/chat';
  let { name, color = '#b7a2d7', size = 36, status, speaking = false }: { name: string; color?: string; size?: number; status?: string; speaking?: boolean } = $props();
</script>
<span class="avatar" class:speaking style:--avatar-color={color} style:--size={`${size}px`} title={name} aria-label={`${name}${status ? `, ${status}` : ''}`}>
  <span>{initials(name)}</span>
  {#if status}<i class="presence {status}"></i>{/if}
</span>
