import { describe, expect, it } from 'vitest';
import { RxFerrixError } from '@rxferrix/client';
import { errorMessage } from '../src/lib/data/errors';

describe('sign-up policy errors', () => {
  it.each([
    ['invite_required', 'Open an invite link to create an account on this server.'],
    ['invalid_invite', 'This invite has expired or reached its limit. Ask for a fresh one.'],
    ['email_not_allowed', 'This email address is not approved for this server. Use an approved address.'],
  ])('explains %s from the authentication service', (code, message) => {
    expect(errorMessage(new RxFerrixError(code, { code }))).toBe(message);
  });

  it('preserves service messages for other authentication failures', () => {
    expect(
      errorMessage(new RxFerrixError('Email or password is incorrect.', { code: 'invalid_credentials' })),
    ).toBe('Email or password is incorrect.');
  });
});
