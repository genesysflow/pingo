import { test, expect } from '@playwright/test';
import { signup, createSpace, send, submitAuth } from './helpers';

test('password sessions, profile, uploads, room management, search and email-code sign-in', async ({
  page,
}) => {
  const errors: string[] = [];
  page.on('pageerror', (e) => errors.push(e.message));
  await signup(page, 'Robin');
  await createSpace(page);
  const roomUrl = page.url();
  const email = await page.evaluate(() => {
    const entry = Object.entries(localStorage).find(([key]) => key.startsWith('rxferrix.auth:'));
    return entry ? (JSON.parse(entry[1]).userId as string) : '';
  });
  expect(email).not.toBe('');
  await page.getByRole('button', { name: 'Your profile', exact: true }).click();
  await page.getByLabel('Your name', { exact: true }).fill('RobinUpdated');
  await page.getByRole('button', { name: 'Save your details' }).click();
  await expect(page.locator('.profile-preview')).toContainText('RobinUpdated');
  await page.getByRole('button', { name: 'Back to your people' }).click();
  await page
    .locator('input[type=file]')
    .first()
    .setInputFiles({
      name: 'notes.txt',
      mimeType: 'text/plain',
      buffer: Buffer.from('Pingo upload from a real browser'),
    });
  await expect(page.locator('.upload-previews')).toContainText('notes.txt');
  await send(page, 'Upload and search check');
  const message = page.getByRole('article').filter({ hasText: 'Upload and search check' });
  const download = message.getByRole('link', { name: /notes.txt/ });
  await expect(download).toBeVisible();
  expect(await (await page.request.get((await download.getAttribute('href')) ?? '')).text()).toBe(
    'Pingo upload from a real browser',
  );
  await message.hover();
  await message.getByRole('button', { name: 'Edit message', exact: true }).click();
  await page.getByRole('textbox', { name: 'Edit message', exact: true }).fill('A searchable edited message');
  await page.getByRole('button', { name: 'Save changes', exact: true }).click();
  await expect(page.getByRole('article').filter({ hasText: 'A searchable edited message' })).toBeVisible();
  await page.getByRole('button', { name: 'Search messages', exact: true }).click();
  await page.getByRole('combobox').fill('searchable');
  await expect(page.getByRole('option').filter({ hasText: 'A searchable edited message' })).toBeVisible();
  await page.keyboard.press('Escape');
  await page.getByRole('button', { name: 'Room settings', exact: true }).click();
  const settings = page.getByRole('dialog', { name: 'Room settings', exact: true });
  await settings.getByLabel('Topic', { exact: true }).fill('Real room settings');
  await settings.getByRole('button', { name: 'Save room', exact: true }).click();
  await expect(page.locator('header')).toContainText('Real room settings');
  await page.getByRole('article').filter({ hasText: 'A searchable edited message' }).hover();
  await page.getByRole('button', { name: 'Delete message', exact: true }).click();
  await page.getByRole('button', { name: 'Delete', exact: true }).click();
  await expect(page.getByText('This message was deleted.', { exact: true })).toBeVisible();
  // A fresh email exercises the real magic-link provider using its dev mailer's six-digit code.
  await page.getByRole('button', { name: 'Open settings', exact: true }).click();
  await page.getByRole('button', { name: 'Sign out', exact: true }).click();
  await page.getByRole('button', { name: 'Email me a magic link instead' }).click();
  const magicEmail = `magic-${crypto.randomUUID()}@pingo.test`;
  await page.getByLabel('Email address', { exact: true }).fill(magicEmail);
  await page.getByRole('button', { name: 'Send me a sign-in link' }).click();
  const response = await page.request.post('/api/action', {
    data: { path: 'dev:last_code', args: { email: magicEmail } },
  });
  const body = await response.json();
  expect(body.status).toBe('success');
  await page.getByLabel('Email code', { exact: true }).fill(body.value);
  await submitAuth(page, page.getByRole('button', { name: 'Verify email code', exact: true }), 'magic/verify');
  await expect(page.getByRole('heading', { name: 'Make a little space.' })).toBeVisible();
  expect(errors).toEqual([]);
});
