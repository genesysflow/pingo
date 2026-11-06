/** Virtualized rows can be mounted offscreen; acknowledge only the visible conversation content. */
export function createReadObserver(markRead: (messages: string[]) => Promise<unknown>) {
  const nodes = new Map<Element, string>();
  const visible = new Map<Element, string>();
  const visibleMessages = () => new Set(visible.values());
  const completed = new Set<string>();
  const inFlight = new Set<string>();
  let observer: IntersectionObserver | undefined;
  let timer: ReturnType<typeof setTimeout> | undefined;
  let disposed = false;

  function schedule(delay = 100) {
    if (
      !disposed &&
      !document.hidden &&
      !timer &&
      [...visibleMessages()].some((id) => !completed.has(id) && !inFlight.has(id))
    )
      timer = setTimeout(() => void flush(), delay);
  }
  async function flush() {
    timer = undefined;
    if (disposed || document.hidden) return;
    const batch = [...visibleMessages()]
      .filter((id) => !completed.has(id) && !inFlight.has(id))
      .slice(0, 100);
    if (!batch.length) return;
    batch.forEach((id) => inFlight.add(id));
    let retry = false;
    try {
      await markRead(batch);
      batch.forEach((id) => completed.add(id));
    } catch {
      retry = true;
    } finally {
      batch.forEach((id) => inFlight.delete(id));
      schedule(retry ? 1000 : 100);
    }
  }
  function visibilityChanged() {
    if (document.hidden) {
      clearTimeout(timer);
      timer = undefined;
    } else schedule();
  }
  return {
    observe(node: HTMLElement, id: string) {
      if (id.startsWith('optimistic:')) return {};
      if (!observer) {
        observer = new IntersectionObserver((entries) => {
          for (const entry of entries) {
            const message = nodes.get(entry.target);
            if (!message) continue;
            if (entry.isIntersecting && entry.intersectionRect.height > 0) visible.set(entry.target, message);
            else visible.delete(entry.target);
          }
          schedule();
        });
        document.addEventListener('visibilitychange', visibilityChanged);
      }
      nodes.set(node, id);
      observer.observe(node);
      return {
        destroy() {
          observer?.unobserve(node);
          nodes.delete(node);
          visible.delete(node);
        },
      };
    },
    destroy() {
      disposed = true;
      clearTimeout(timer);
      observer?.disconnect();
      document.removeEventListener('visibilitychange', visibilityChanged);
      nodes.clear();
      visible.clear();
    },
  };
}
