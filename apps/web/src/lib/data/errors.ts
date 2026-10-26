const messages: Record<string, string> = {
  not_found: 'That item is no longer available.',
  forbidden: 'You do not have permission to do that.',
  unauthenticated: 'Please sign in to continue.',
  invalid_name: 'Choose a name between 1 and 32 characters.',
  name_taken: 'That name is already in use. Try another.',
  invalid_invite: 'This invite has expired or reached its limit. Ask for a fresh one.',
  invite_required: 'Open an invite link to create an account on this server.',
  email_not_allowed: 'This email address is not approved for this server. Use an approved address.',
  rate_limited: 'A little too fast. Please try again in a moment.',
  too_long: 'That is too long. Messages can contain up to 4,000 characters.',
  ConnectionLost: 'Connection lost. Reconnecting…',
};
export function errorMessage(error: unknown): string {
  if (error && typeof error === 'object') {
    const e = error as { code?: string; message?: string };
    return (e.code && messages[e.code]) || e.message || 'Something went wrong. Please try again.';
  }
  return 'Something went wrong. Please try again.';
}
