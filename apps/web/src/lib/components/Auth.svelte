<script lang="ts">
  import { errorMessage } from '../data/errors';
  import { onMount } from 'svelte';
  import type { AuthProviders } from '@rxferrix/client';
  import { api, useAuth, useQuery, useMutation, isDemo } from '../data';
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
    busy = $state(false),
    code = $state(''),
    mfaToken = $state('');
  let providers = $state<AuthProviders | null>(null);
  onMount(() => {
    void auth
      .providers()
      .then((p) => (providers = p))
      .catch((e) => (error = errorMessage(e)));
  });
  async function finish() {
    if (inviteCode) enter(await accept({ code: inviteCode }));
    else enter();
  }
  async function verifyCode(e: SubmitEvent) {
    e.preventDefault();
    busy = true;
    error = '';
    try {
      const result = await auth.verifyCode({ email, code: code.trim() });
      if (result.mfaRequired) {
        mfaToken = result.mfaToken;
        sent = false;
        code = '';
      } else await finish();
    } catch (e) {
      error = errorMessage(e);
    } finally {
      busy = false;
    }
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
      if (mfaToken) {
        await auth.verifyMfa(mfaToken, code.trim());
        await finish();
        return;
      }
      if (magic) {
        await auth.requestMagicLink({ email, redirectTo: inviteCode ? location.href : location.origin });
        sent = true;
      } else {
        if (signup) {
          const result = await auth.signUp({ email, password, name });
          if (result.verificationRequired) {
            error = 'Check your email to verify your address, then sign in.';
            return;
          }
        } else {
          const result = await auth.signIn({ email, password });
          if (result.mfaRequired) {
            mfaToken = result.mfaToken;
            code = '';
            return;
          }
        }
        await finish();
      }
    } catch (e) {
      error = errorMessage(e);
    } finally {
      busy = false;
    }
  }
</script>

<div class="auth-page">
  <div class="auth-story">
    <a class="auth-brand" href="/"><img src="/pingo.svg" alt="" />Pingo</a>
    <div class="auth-pitch">
      <h1>Chat, voice and screen sharing for small teams.</h1>
      <ul class="auth-specs">
        <li><span>Voice</span>Opus at 48 kHz, stereo Studio mode</li>
        <li><span>Noise</span>DeepFilterNet3 on every microphone</li>
        <li><span>Screen</span>Sharp text up to 4K, or 1080p60 motion</li>
        <li><span>Server</span>One binary you run yourself</li>
      </ul>
    </div>
    <div class="auth-meter" aria-hidden="true">
      {#each Array(28) as _, i}<i style:--h={`${18 + Math.abs(Math.sin(i * 1.7) * 62)}%`} style:--d={`${(i % 7) * 0.13}s`}></i>{/each}
    </div>
  </div>
  <main class="auth-form-side">
    <div class="auth-form">
      {#if inviteCode && preview.data === null}<h2>This invite has expired</h2>
        <p class="settings-description">Ask whoever sent it for a new link.</p>
        <button
          class="button primary"
          onclick={() => {
            history.replaceState({}, '', '/');
            enter();
          }}>{isDemo ? 'Explore the demo' : 'Back to Pingo'}</button
        >
      {:else if inviteCode && auth.isAuthenticated}<span class="auth-kicker">Invitation</span>
        <h2>Join {preview.data?.spaceName ?? 'this space'}</h2>
        <p class="settings-description">
          {preview.data?.memberCount ?? 0}
          {preview.data?.memberCount === 1 ? 'member' : 'members'}
        </p>
        <button
          class="button primary full"
          disabled={!preview.data}
          onclick={() => finish().catch((e) => (error = errorMessage(e)))}
          >Join {preview.data?.spaceName}</button
        >
        {#if isDemo}<p class="muted tiny">Interactive demo. No account or email needed.</p>{/if}
        {#if error}<p class="error" role="alert">{error}</p>{/if}
      {:else if sent}<h2>Check your email</h2>
        <p class="settings-description">
          {#if isDemo}In the live app, a sign-in link would arrive at <strong>{email}</strong>. This demo
            keeps everything local.{:else}We sent a sign-in link and code to <strong>{email}</strong>.{/if}
        </p>
        {#if !isDemo}<form onsubmit={verifyCode}>
            <label class="field"
              >Email code<input
                bind:value={code}
                autocomplete="one-time-code"
                inputmode="numeric"
                required
              /></label
            >
            {#if error}<p class="error" role="alert">{error}</p>{/if}
            <button class="button primary full" disabled={busy}>Verify email code</button>
          </form>{/if}
        <!-- A local shortcut exists only in demo mode; live links are consumed by the auth redirect handler. -->{#if isDemo}<button
            class="button primary full"
            onclick={async () => {
              await auth.verifyMagicLink('demo');
              await finish();
            }}>Try the demo sign-in link</button
          >{/if}<button class="text-button" onclick={() => (sent = false)}>Use another email</button>
      {:else}<h2>{signup ? 'Create an account' : 'Sign in'}</h2>
        {#if inviteCode}<p class="settings-description">
            to join {preview.data?.spaceName ?? 'this space'}
          </p>{/if}
        {#if providers?.oauth.length}<div class="oauth-buttons">
            {#each providers.oauth as provider}<button class="button" onclick={() => oauth(provider)}>
                <Icon
                  name={provider === 'github' ? 'github' : provider === 'discord' ? 'headphones' : 'globe'}
                  size={17}
                />
                {provider === 'github'
                  ? 'GitHub'
                  : provider === 'discord'
                    ? 'Discord'
                    : provider === 'google'
                      ? 'Google'
                      : provider}
              </button>{/each}
          </div>
          <div class="auth-divider"><span>or with email</span></div>{/if}
        <form onsubmit={submit}>
          {#if mfaToken}<label class="field"
              >Authenticator or recovery code<input
                bind:value={code}
                autocomplete="one-time-code"
                required
              /></label
            >{:else}{#if signup}<label class="field"
                >Your name<input
                  autocomplete="name"
                  bind:value={name}
                  required
                  maxlength="32"
                  placeholder="Ada Lovelace"
                /></label
              >{/if}<label class="field"
              >Email address<input
                type="email"
                autocomplete="email"
                required
                bind:value={email}
                placeholder="you@example.com"
              /></label
            >{#if !magic}<label class="field"
                >Password<input
                  type="password"
                  autocomplete={signup ? 'new-password' : 'current-password'}
                  minlength="8"
                  bind:value={password}
                  required
                  placeholder="At least 8 characters"
                /></label
              >{/if}{/if}{#if error}<p class="error" role="alert">{error}</p>{/if}<button
            class="button primary full"
            disabled={busy}
            >{mfaToken
              ? 'Verify and sign in'
              : magic
                ? 'Send me a sign-in link'
                : signup
                  ? 'Create account'
                  : 'Sign in'}</button
          >
        </form>
        {#if providers?.magicLink}<button class="magic-link" onclick={() => (magic = !magic)}
            ><Icon name={magic ? 'lock' : 'mail'} size={15} />{magic
              ? 'Use a password instead'
              : 'Email me a magic link instead'}</button
          >
        {/if}{#if providers?.signups}<p class="auth-switch">
            {signup ? 'Have an account?' : 'New to Pingo?'}<button onclick={() => (signup = !signup)}
              >{signup ? 'Sign in' : 'Create an account'}</button
            >
          </p>
        {/if}{#if isDemo}<p class="demo-note">
            Local demo: use any email and a password of 6+ characters. OAuth buttons simulate sign-in; no email
            is sent.
          </p>{/if}{/if}
    </div>
  </main>
</div>
