import { test, expect } from '@playwright/test';
import { signup, createSpace, invite, accept, send } from './helpers';

test('inbox clears viewed messages and keeps unopened thread replies pending', async ({ browser }) => {
  const contexts = await Promise.all([browser.newContext(), browser.newContext()]);
  const [alice, bob] = await Promise.all(contexts.map((context) => context.newPage()));
  const errors: string[] = [];
  for (const page of [alice, bob]) page.on('pageerror', (error) => errors.push(error.message));
  try {
    await signup(alice, 'InboxAlice');
    await signup(bob, 'InboxBob');
    await createSpace(alice);
    await accept(bob, await invite(alice));
    const inbox = () => bob.getByRole('button', { name: 'Inbox', exact: true }).first().click();
    await inbox();
    await send(alice, 'Please review this @InboxBob');
    await expect(bob.locator('.inbox-card')).toHaveCount(1);
    await bob.locator('.inbox-card').getByRole('button', { name: 'Open', exact: true }).click();
    await expect(
      bob.getByRole('article').filter({ hasText: 'Please review this @InboxBob' }),
    ).toBeInViewport();
    await expect(bob.getByRole('button', { name: 'Inbox', exact: true }).locator('.count')).toHaveCount(0);
    await inbox();
    await expect(bob.locator('.inbox-card')).toHaveCount(0);
    await bob.reload();
    await expect(bob.getByRole('heading', { name: 'You’re all caught up' })).toBeVisible();

    await bob.getByRole('button', { name: 'Back to rooms', exact: true }).click();
    await send(bob, 'A thread that still needs attention');
    const root = alice.getByRole('article').filter({ hasText: 'A thread that still needs attention' });
    await inbox();
    await root.hover();
    await root.getByRole('button', { name: 'Reply in thread', exact: true }).click();
    const aliceThread = alice.getByRole('complementary', { name: 'Thread', exact: true });
    await aliceThread.getByRole('textbox').fill('Unread thread reply');
    await aliceThread.getByRole('textbox').press('Enter');
    await expect(bob.locator('.inbox-card').filter({ hasText: 'Unread thread reply' })).toBeVisible();
    await bob
      .getByRole('navigation', { name: 'Chat rooms', exact: true })
      .getByRole('button', { name: /^general(?: \d+)?$/ })
      .click();
    await expect(
      bob.getByRole('article').filter({ hasText: 'A thread that still needs attention' }),
    ).toBeInViewport();
    await inbox();
    await expect(bob.locator('.inbox-card').filter({ hasText: 'Unread thread reply' })).toBeVisible();
    await bob.locator('.inbox-card').getByRole('button', { name: 'Open thread', exact: true }).click();
    await expect(
      bob
        .getByRole('complementary', { name: 'Thread', exact: true })
        .getByText('Unread thread reply', { exact: true }),
    ).toBeInViewport();
    await expect(bob.getByRole('button', { name: 'Inbox', exact: true }).locator('.count')).toHaveCount(0);
    await bob.getByRole('button', { name: 'Close thread', exact: true }).click();
    await inbox();
    await expect(bob.locator('.inbox-card')).toHaveCount(0);

    await alice.getByRole('button', { name: 'Close thread', exact: true }).click();
    await alice.getByRole('button', { name: 'Show members', exact: true }).click();
    await alice
      .getByRole('complementary', { name: 'Members', exact: true })
      .getByRole('button')
      .filter({ hasText: 'InboxBob' })
      .click();
    const dm = alice.getByRole('textbox', { name: 'Message Direct message', exact: true });
    for (const body of ['First private message', 'Second private message', 'Third private message']) {
      await dm.fill(body);
      await dm.press('Enter');
    }
    await expect(bob.locator('.inbox-card')).toHaveCount(3);
    await bob
      .getByRole('navigation', { name: 'Direct messages', exact: true })
      .getByRole('button')
      .filter({ hasText: 'InboxAlice' })
      .click();
    await expect(bob.getByRole('article').filter({ hasText: 'Third private message' })).toBeInViewport();
    await expect(bob.getByRole('button', { name: 'Inbox', exact: true }).locator('.count')).toHaveCount(0);
    await inbox();
    await expect(bob.locator('.inbox-card')).toHaveCount(0);
    expect(errors).toEqual([]);
  } finally {
    await Promise.all(contexts.map((context) => context.close()));
  }
});
