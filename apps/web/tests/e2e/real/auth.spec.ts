import { test, expect } from '@playwright/test';
import { signup, createSpace, invite, submitAuth } from './helpers';

test('password signup without an invite returns to the server origin', async ({ page }) => {
  const request = page.waitForRequest(
    (candidate) => new URL(candidate.url()).pathname === '/api/auth/signup',
  );
  await signup(page, 'NoInvite');
  expect((await request).postDataJSON().redirectTo).toBe(new URL(page.url()).origin);
});

test('password signup carries the invitation URL and joins its space', async ({ page, browser }) => {
  await signup(page, 'InviteOwner');
  await createSpace(page);
  const link = await invite(page);
  const guestContext = await browser.newContext();
  try {
    const guest = await guestContext.newPage();
    await guest.goto(`${link}?source=email`);
    await guest.getByRole('button', { name: 'Create an account', exact: true }).click();
    await guest.getByLabel('Your name', { exact: true }).fill('InvitedGuest');
    const email = `invited-${crypto.randomUUID()}@pingo.test`;
    await guest.getByLabel('Email address', { exact: true }).fill(email);
    await guest.getByLabel('Password', { exact: true }).fill('correct-horse-pingo');
    const redirectTo = guest.url();
    const request = guest.waitForRequest(
      (candidate) => new URL(candidate.url()).pathname === '/api/auth/signup',
    );
    await submitAuth(guest, guest.getByRole('button', { name: 'Create account', exact: true }), 'signup');
    expect((await request).postDataJSON()).toEqual({
      email,
      password: 'correct-horse-pingo',
      name: 'InvitedGuest',
      redirectTo,
    });
    await expect(guest.getByRole('heading', { name: 'general', exact: true })).toBeVisible();
  } finally {
    await guestContext.close();
  }
});
