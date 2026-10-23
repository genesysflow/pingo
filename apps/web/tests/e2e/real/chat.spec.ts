import { test, expect } from '@playwright/test';
import { signup, createSpace, invite, accept, send } from './helpers';
test('real auth, invites, live chat, threads, reactions, unread and mentions', async ({ browser }) => {
  const a = await browser.newContext(),
    b = await browser.newContext();
  const alice = await a.newPage(),
    bob = await b.newPage();
  const errors: string[] = [];
  for (const page of [alice, bob]) page.on('pageerror', (e) => errors.push(e.message));
  try {
    await signup(alice, 'Alice');
    await signup(bob, 'Bob');
    await expect(alice.getByRole('button', { name: 'Google', exact: true })).toHaveCount(0);
    const roomUrl = await createSpace(alice);
    await accept(bob, await invite(alice));
    await send(alice, 'Hello from the real backend');
    const root = bob.getByRole('article').filter({ hasText: 'Hello from the real backend' });
    await expect(root).toBeVisible();
    await root.hover();
    await root.getByRole('button', { name: 'React to message' }).click();
    await root.getByRole('button', { name: 'React 💜', exact: true }).click();
    await expect(alice.getByRole('button', { name: '💜, 1 reactions', exact: true })).toBeVisible();
    await root.hover();
    await root.getByRole('button', { name: 'Reply in thread', exact: true }).click();
    const thread = bob.getByRole('complementary', { name: 'Thread', exact: true });
    await thread.getByRole('textbox').fill('A real thread reply');
    await thread.getByRole('textbox').press('Enter');
    await expect(alice.getByRole('button', { name: /1 replies/ })).toBeVisible();
    await bob.screenshot({ path: 'screenshots/real-thread.png', animations: 'disabled' });
    await bob.getByRole('button', { name: 'Close thread' }).click();
    await bob.getByRole('button', { name: 'Catch up', exact: false }).first().click();
    await send(alice, 'A new update for @Bob');
    await expect(bob.locator('.room-link.unread')).toHaveCount(1);
    await expect(bob.locator('.inbox-card').filter({ hasText: 'A new update for @Bob' })).toBeVisible();
    await bob.screenshot({ path: 'screenshots/real-inbox.png', animations: 'disabled' });
    await bob.goto(roomUrl);
    await expect(bob.getByRole('article').filter({ hasText: 'A new update for @Bob' })).toBeVisible();
    await expect(bob.locator('.room-link.unread')).toHaveCount(0);
    await bob.reload();
    await expect(bob.getByRole('heading', { name: 'general', exact: true })).toBeVisible();
    await alice.screenshot({ path: 'screenshots/real-chat.png', animations: 'disabled' });
    await bob.getByRole('button', { name: 'Catch up', exact: false }).first().click();
    await bob.getByRole('button', { name: 'Mark all done', exact: false }).click();
    await expect(bob.locator('.inbox-card')).toHaveCount(0);
    await bob.getByRole('button', { name: 'Back to your people', exact: false }).click();
    await bob.getByRole('button', { name: 'Show members', exact: true }).click();
    await bob
      .getByRole('complementary', { name: 'Members', exact: true })
      .getByRole('button')
      .filter({ hasText: 'Alice' })
      .click();
    await bob.getByRole('textbox', { name: 'Message Direct message', exact: true }).fill('A private hello');
    await bob.getByRole('button', { name: 'Send message', exact: true }).click();
    await expect(alice.getByRole('navigation', { name: 'Direct messages', exact: true })).toContainText(
      'Bob',
    );
    await alice.getByRole('navigation', { name: 'Direct messages', exact: true }).getByRole('button').click();
    await expect(alice.getByRole('article').filter({ hasText: 'A private hello' })).toBeVisible();
    expect(errors).toEqual([]);
  } finally {
    await a.close();
    await b.close();
  }
});

test('real message history loads older cursor pages without unbounded DOM rows', async ({ page }) => {
  await signup(page, 'History');
  await createSpace(page);
  for (let i = 0; i < 80; i++) await send(page, `History message ${i}`);
  await expect(page.locator('[data-message-id^="optimistic:"]')).toHaveCount(0);
  const log = page.getByRole('log');
  await log.evaluate((node) => (node.scrollTop = 0));
  await page.getByRole('button', { name: 'Earlier conversations' }).click();
  await expect(page.getByRole('button', { name: 'Earlier conversations' })).toHaveCount(0);
  await log.press('Control+Home');
  await expect(page.getByRole('article').filter({ hasText: 'History message 0' }).first()).toBeVisible();
  expect(await page.locator('.virtual-row').count()).toBeLessThan(80);
});
