<script lang="ts">
  import Flourish from './Flourish.svelte';
  import { api, useAuth, useQuery, useMutation, isDemo } from '../data';
  import Avatar from './Avatar.svelte';
  import Icon from './Icon.svelte';
  let { inviteCode, enter }: { inviteCode?: string; enter: (space?: string) => void } = $props();
  const auth = useAuth(),
    preview = useQuery(api.invites.preview, () => (inviteCode ? { code: inviteCode } : 'skip')),
    accept = useMutation(api.invites.accept);
  let signup = $state(location.pathname === '/signup'),
    magic = $state(false),
    sent = $state(false),
    email = $state(''),
    password = $state(''),
    name = $state(''),
    error = $state(''),
    busy = $state(false);
  async function finish() {
    if (inviteCode) enter(await accept({ code: inviteCode }));
    else enter();
  }
  async function oauth(provider: string) {
    auth.signInWithOAuth(provider, { redirectTo: inviteCode ? location.href : location.origin });
    if (auth.isAuthenticated) await finish();
  }
  async function submit(e: SubmitEvent) {
    e.preventDefault();
    error = '';
    busy = true;
    try {
      if (magic) {
        await auth.requestMagicLink({ email, redirectTo: inviteCode ? location.href : location.origin });
        sent = true;
      } else {
        if (signup) await auth.signUp({ email, password, name });
        else await auth.signIn({ email, password });
        await finish();
      }
    } catch (e) {
      error = (e as Error).message;
    } finally {
      busy = false;
    }
  }
</script>

<div class="auth-page">
  <div class="auth-story">
    <a class="auth-brand" href="/"><img src="/pingo.svg" alt="" />pingo<span>a little closer.</span></a>
    <div class="auth-art">
      <span class="auth-star"><Flourish /></span><span class="auth-orbit orbit-one"></span><span
        class="auth-orbit orbit-two"
      ></span><span class="auth-face face-one"
        ><Avatar name="Maya Chen" color="#edb18e" size={82} /><i>hey, you’re here.</i></span
      ><span class="auth-face face-two"><Avatar name="Leo Park" color="#a0c4af" size={64} /></span><span
        class="auth-face face-three"><Avatar name="Sofia Reyes" color="#e6a4ba" size={62} /></span
      ><span class="auth-small-star">✧</span>
    </div>
    <h1>Your people.<br />A little closer.</h1>
    <p>For the big ideas, the little updates,<br />and the conversations in between.</p>
    <span class="auth-story-footer"
      ><Icon name="leaf" size={14} />A little less noise. A little more together.</span
    >
  </div>
  <main class="auth-form-side">
    <div class="auth-theme-note">YOUR NEXT GOOD CONVERSATION STARTS HERE</div>
    <div class="auth-form">
      {#if inviteCode && preview.data === null}<span class="empty-icon"><Icon name="mail" size={32} /></span>
        <h2>This little door has closed.</h2>
        <p class="settings-description">That invite has expired. Ask your person for a fresh one.</p>
        <button
          class="button primary"
          onclick={() => {
            history.replaceState({}, '', '/');
            enter();
          }}>Explore the demo</button
        >
      {:else if inviteCode && auth.isAuthenticated}<span class="invite-flower"><Flourish /></span><span
          class="eyebrow">A PLACE FOR YOU</span
        >
        <h2>Pull up a chair.</h2>
        <p class="settings-description">
          You’ve been invited to <strong>{preview.data?.spaceName}</strong>. A few good people are already
          here.
        </p>
        <div class="invite-people">
          <div class="avatar-stack">
            <Avatar name="Maya Chen" color="#edb18e" /><Avatar name="Leo Park" color="#a0c4af" /><Avatar
              name="Sofia Reyes"
              color="#e6a4ba"
            />
          </div>
          <span>{preview.data?.memberCount} people making good things.</span>
        </div>
        <button class="button primary full" onclick={finish}
          >Join {preview.data?.spaceName}<Icon name="arrowRight" size={17} /></button
        >
        <p class="muted tiny">
          {isDemo
            ? 'Interactive demo · no account or email is required.'
            : 'A little room to make something together.'}
        </p>
      {:else if sent}<span class="empty-icon"><Icon name="mail" size={32} /></span>
        <h2>A little note for you.</h2>
        <p class="settings-description">
          {#if isDemo}In the live app, a sign-in link would arrive at <strong>{email}</strong>. This demo
            keeps everything local.{:else}We’ve sent a sign-in link to <strong>{email}</strong>. Open it to
            come on in.{/if}
        </p>
        <!-- A local shortcut exists only in demo mode; live links are consumed by the auth redirect handler. -->{#if isDemo}<button
            class="button primary full"
            onclick={async () => {
              await auth.verifyMagicLink('demo');
              await finish();
            }}>Try the demo sign-in link<Icon name="arrowRight" size={17} /></button
          >{/if}<button class="text-button" onclick={() => (sent = false)}>Use another email</button>
      {:else}<span class="eyebrow">{signup ? 'MAKE YOURSELF AT HOME' : 'GOOD TO HAVE YOU HERE'}</span>
        <h2>
          {signup ? 'A fresh little start.' : inviteCode ? 'Your people are waiting.' : 'Welcome back.'}
        </h2>
        <p class="settings-description">
          {inviteCode
            ? `Sign in to join ${preview.data?.spaceName ?? 'your space'}.`
            : signup
              ? 'There’s always room for one more good person.'
              : 'Settle in. There’s a little catching up to do.'}
        </p>
        <div class="oauth-buttons">
          <button class="button" onclick={() => oauth('google')}><Icon name="globe" size={17} />Google</button
          ><button class="button" onclick={() => oauth('github')}
            ><Icon name="github" size={17} />GitHub</button
          ><button class="button" onclick={() => oauth('discord')}
            ><Icon name="headphones" size={17} />Discord</button
          >
        </div>
        <div class="auth-divider"><span>or a little more familiar</span></div>
        <form onsubmit={submit}>
          {#if signup}<label class="field"
              >Your name<input
                autocomplete="name"
                bind:value={name}
                required
                maxlength="32"
                placeholder="What should we call you?"
              /></label
            >{/if}<label class="field"
            >Email address<input
              type="email"
              autocomplete="email"
              required
              bind:value={email}
              placeholder="you@somewhere.good"
            /></label
          >{#if !magic}<label class="field"
              >Password<input
                type="password"
                autocomplete={signup ? 'new-password' : 'current-password'}
                minlength="6"
                bind:value={password}
                required
                placeholder="Your secret little phrase"
              /></label
            >{/if}{#if error}<p class="error" role="alert">{error}</p>{/if}<button
            class="button primary full"
            disabled={busy}
            >{magic ? 'Send me a sign-in link' : signup ? 'Make yourself at home' : 'Come on in'}<Icon
              name="arrowRight"
              size={17}
            /></button
          >
        </form>
        <button class="magic-link" onclick={() => (magic = !magic)}
          ><Icon name={magic ? 'lock' : 'mail'} size={15} />{magic
            ? 'Use a password instead'
            : 'Email me a magic link instead'}</button
        >
        <p class="auth-switch">
          {signup ? 'Already one of us?' : 'New around here?'}<button onclick={() => (signup = !signup)}
            >{signup ? 'Sign in' : 'Create an account'}<Icon name="upRight" size={13} /></button
          >
        </p>
        {#if isDemo}<p class="demo-note">
            <Icon name="sparkles" size={14} />This is a local demo. Use any email and a 6-character password.<br
            />OAuth buttons simulate sign-in; no email is sent.
          </p>{/if}{/if}
    </div>
    <span class="auth-legal">A good place to be yourself.</span>
  </main>
</div>
