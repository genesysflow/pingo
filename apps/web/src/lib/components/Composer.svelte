<script lang="ts">
  import { onDestroy } from 'svelte';
  import { api, useMutation, useQuery, uploadFile, insertAtTop, type MessageView } from '../data';
  import { emoji, expandEmoji } from '../logic/chat';
  import Icon from './Icon.svelte';
  let { room, name, space, threadRoot, onSent, command }: { room: string; name: string; space: string; threadRoot?: string; onSent?: () => void; command?: (cmd: string) => void } = $props();
  let text = $state(''), error = $state(''), uploading = $state(false), dragOver = $state(false), picker = $state(false), selected = $state(0);
  let attachments = $state<{ id: string; name: string; preview: string; image: boolean }[]>([]);
  let input: HTMLTextAreaElement, fileInput: HTMLInputElement;
  let lastTyping = 0;
  const send = useMutation(api.messages.send), startTyping = useMutation(api.typing.start), generateUrl = useMutation(api.files.generateUploadUrl);
  const me = useQuery(api.profiles.me);
  const members = useQuery(api.spaces.members, () => ({ space }));
  const token = $derived(text.match(/(?:^|\s)([@:/][\w-]*)$/)?.[1] ?? '');
  const suggestions = $derived(token.startsWith('@') ? (members.data ?? []).filter(p => p.name.toLowerCase().includes(token.slice(1).toLowerCase())).map(p => ({ label: p.name, value: `@${p.name.split(' ')[0]} `, detail: 'Mention someone', icon: '@' })) : token.startsWith(':') ? Object.entries(emoji).filter(([key]) => key.includes(token.slice(1))).map(([key, val]) => ({ label: `:${key}:`, value: `${val} `, detail: '', icon: val })) : token.startsWith('/') ? [{ label: '/call', value: '/call', detail: 'Pull up a chair', icon: '↗' }, { label: '/share', value: '/share', detail: 'Share your screen', icon: '↗' }, { label: '/status', value: '/status', detail: 'Set a little status', icon: '◉' }, { label: '/shrug', value: '¯\\_(ツ)_/¯ ', detail: 'Sometimes words aren’t enough', icon: '¯' }].filter(s => s.label.includes(token)) : []);
  function choose(i: number) { const s = suggestions[i]; if (s) { text = text.slice(0, text.length - token.length) + s.value; selected = 0; input.focus(); } }
  function changed() { selected = 0; if (Date.now() - lastTyping > 2000 && text.trim()) { lastTyping = Date.now(); void startTyping({ room }); } }
  async function submit() {
    if (uploading || (!text.trim() && !attachments.length)) return;
    error = '';
    if (/^\/(call|share|status)$/.test(text.trim())) { command?.(text.trim().slice(1)); text = ''; return; }
    const body = expandEmoji(text), files = attachments;
    text = ''; attachments = [];
    const optimistic: MessageView = { _id: `optimistic:${crypto.randomUUID()}`, _creationTime: Date.now(), room, author: me.data ?? { user: 'you', name: 'You', color: '#b9a2e7' }, body, threadRoot, replyCount: 0, reactions: [], attachments: files.map(f => ({ id: f.id, name: f.name, url: f.preview, size: 0, contentType: f.image ? 'image/png' : 'application/octet-stream' })), deleted: false };
    const mutate = send.withOptimisticUpdate((store, args) => {
      if (args.threadRoot) { const current = store.getQuery(api.messages.thread, { root: args.threadRoot }); if (current) store.setQuery(api.messages.thread, { root: args.threadRoot }, { ...current, root: { ...current.root, replyCount: current.root.replyCount + 1 }, replies: [...current.replies, optimistic] }); }
      else insertAtTop({ paginatedQuery: api.messages.list, argsToMatch: { room: args.room }, localQueryStore: store, item: optimistic });
    });
    try { await mutate({ room, body, threadRoot, attachments: files.map(f => f.id) }); onSent?.(); }
    catch (e) { text = body; attachments = files; error = (e as Error).message; }
    input.focus();
  }
  async function attach(files: FileList | File[]) {
    error = ''; if (attachments.length + files.length > 10) { error = 'Up to 10 files per message, please.'; return; }
    uploading = true;
    try { for (const f of Array.from(files)) { if (f.size > 20 * 1024 * 1024) throw new Error('Choose files smaller than 20 MB.'); const url = await generateUrl(); const id = await uploadFile(url, f); attachments.push({ id, name: f.name, preview: URL.createObjectURL(f), image: f.type.startsWith('image/') }); } } catch (e) { error = (e as Error).message; } finally { uploading = false; }
  }
  function removeAttachment(i: number) { URL.revokeObjectURL(attachments[i].preview); attachments.splice(i, 1); }
  onDestroy(() => attachments.forEach(a => URL.revokeObjectURL(a.preview)));
  function format(before: string, after = before) { const start = input.selectionStart, end = input.selectionEnd; text = text.slice(0, start) + before + text.slice(start, end) + after + text.slice(end); input.focus(); requestAnimationFrame(() => input.setSelectionRange(start + before.length, end + before.length)); }
  function keydown(e: KeyboardEvent) {
    if ((e.ctrlKey || e.metaKey) && ['b', 'i'].includes(e.key)) { e.preventDefault(); format(e.key === 'b' ? '**' : '*'); return; }
    if (suggestions.length && ['ArrowUp', 'ArrowDown', 'Tab'].includes(e.key)) { e.preventDefault(); if (e.key === 'Tab') choose(selected); else selected = (selected + (e.key === 'ArrowUp' ? -1 : 1) + suggestions.length) % suggestions.length; return; }
    if (e.key === 'Enter' && !e.shiftKey && !e.isComposing) { e.preventDefault(); if (suggestions.length && !text.startsWith('/')) choose(selected); else void submit(); }
    if (e.key === 'Escape') { picker = false; text += ' '; }
  }
</script>
<div class="composer-wrap">
  <div class="composer" class:drag-over={dragOver} role="group" aria-label="Message composer" ondragover={e => { e.preventDefault(); dragOver = true; }} ondragleave={() => dragOver = false} ondrop={e => { e.preventDefault(); dragOver = false; if (e.dataTransfer?.files) void attach(e.dataTransfer.files); }}>
    {#if suggestions.length}<div class="autocomplete" role="listbox" aria-label="Suggestions">{#each suggestions.slice(0, 8) as s, i}<button class:chosen={selected === i} role="option" aria-selected={selected === i} onmousedown={e => e.preventDefault()} onclick={() => choose(i)}><span class="suggestion-icon">{s.icon}</span><strong>{s.label}</strong><small>{s.detail}</small>{#if selected === i}<kbd>Tab</kbd>{/if}</button>{/each}</div>{/if}
    {#if picker}<div class="emoji-picker composer-emoji" role="group" aria-label="Insert emoji">{#each Object.values(emoji) as em}<button aria-label={`Insert ${em}`} onclick={() => { text += em; picker = false; input.focus(); }}>{em}</button>{/each}</div>{/if}
    {#if attachments.length}<div class="upload-previews">{#each attachments as f, i}<div>{#if f.image}<img src={f.preview} alt={f.name}/>{:else}<Icon name="file"/>{/if}<span>{f.name}</span><button aria-label={`Remove ${f.name}`} onclick={() => removeAttachment(i)}><Icon name="close" size={14}/></button></div>{/each}</div>{/if}
    <textarea bind:this={input} bind:value={text} aria-label={threadRoot ? 'Reply to thread' : `Message ${name}`} placeholder={threadRoot ? 'Keep the conversation going…' : `A little something for #${name}…`} maxlength="4000" rows="2" oninput={changed} onkeydown={keydown} onpaste={e => { if (e.clipboardData?.files.length) { e.preventDefault(); void attach(e.clipboardData.files); } }}></textarea>
    <div class="composer-toolbar"><div><button class="icon-button" aria-label="Attach files" onclick={() => fileInput.click()}><Icon name="plus" size={19}/></button><span class="toolbar-line"></span><button class="icon-button" aria-label="Bold" onclick={() => format('**')}><Icon name="bold" size={16}/></button><button class="icon-button" aria-label="Italic" onclick={() => format('*')}><Icon name="italic" size={16}/></button><button class="icon-button" aria-label="Insert code" onclick={() => format('`')}><Icon name="code" size={17}/></button><button class="icon-button" aria-label="Insert emoji" onclick={() => picker = !picker}><Icon name="smile" size={18}/></button><button class="icon-button" aria-label="Mention someone" onclick={() => { text += ' @'; input.focus(); }}>@</button></div><button class="send-button" disabled={uploading || (!text.trim() && !attachments.length)} aria-label="Send message" onclick={submit}>{#if uploading}Uploading…{:else}<Icon name="arrowUp" size={18}/>{/if}</button></div>
    <input type="file" multiple hidden bind:this={fileInput} onchange={() => { if (fileInput.files) void attach(fileInput.files); fileInput.value = ''; }}/>
  </div>
  <div class="composer-hint"><span><strong>Enter</strong> to send · <strong>Shift + Enter</strong> for a new line</span><span>Make a little space for kindness <Icon name="sparkles" size={12}/></span></div>
  {#if error}<p class="error" role="alert">{error}</p>{/if}
</div>
