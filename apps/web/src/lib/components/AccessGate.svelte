<script lang="ts">
  import { api, useAuth, useMutation, errorMessage, type AccessStatus } from '../data';
  let { status, joined }: { status: AccessStatus; joined: (space: string) => void } = $props();
  const auth = useAuth();
  const accept = useMutation(api.invites.accept);
  let invite = $state(''),
    code = $state(''),
    error = $state(''),
    note = $state(''),
    busy = $state(false);
  async function run(task: () => Promise<void>) {
    error = '';
    note = '';
    busy = true;
    try {
      await task();
    } catch (e) {
      error = errorMessage(e);
    } finally {
      busy = false;
    }
  }
</script>

<main class="onboarding">
  <div class="onboarding-card">
    <img src="/pingo.svg" width="36" height="36" alt="Pingo" />
    {#if status.admission === 'verifyEmail'}<h1>Confirm your email</h1>
      <p class="settings-description">
        This server only accepts certain addresses. Enter the code we sent to <strong>{status.email}</strong>.
      </p>
      <form
        onsubmit={(e) => {
          e.preventDefault();
          void run(() => auth.verifyEmail({ email: status.email ?? '', code: code.trim() }));
        }}
      >
        <label class="field"
          >Verification code<input
            bind:value={code}
            autocomplete="one-time-code"
            inputmode="numeric"
            required
          /></label
        >
        <button class="button primary full" disabled={busy}>Confirm</button>
      </form>
      <button
        class="text-button"
        disabled={busy}
        onclick={() =>
          run(async () => {
            await auth.requestEmailVerification({ email: status.email ?? '', redirectTo: location.origin });
            note = 'A new code is on its way.';
          })}>Send a new code</button
      >
    {:else if status.admission === 'emailNotAllowed'}<h1>This address can’t join</h1>
      <p class="settings-description">
        <strong>{status.email ?? 'Your account'}</strong> isn’t on this server’s list of allowed addresses. Sign in
        with another account, or ask the server’s admin to add you.
      </p>
    {:else}<h1>You need an invite</h1>
      <p class="settings-description">
        This server is invite-only. Ask someone in a space for an invite link and paste it here.
      </p>
      <form
        onsubmit={(e) => {
          e.preventDefault();
          void run(async () => joined(await accept({ code: invite.trim().split('/').at(-1)! })));
        }}
      >
        <label class="field">Invite link or code<input bind:value={invite} required /></label>
        <button class="button primary full" disabled={busy}>Join space</button>
      </form>
    {/if}
    {#if note}<p class="settings-description" role="status">{note}</p>{/if}
    {#if error}<p class="error" role="alert">{error}</p>{/if}
    <p class="access-account">
      Signed in{status.email ? ` as ${status.email}` : ''} ·
      <button class="text-button" onclick={() => auth.signOut()}>Sign out</button>
    </p>
  </div>
</main>
