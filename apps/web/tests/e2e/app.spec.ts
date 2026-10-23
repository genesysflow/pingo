import { expect, test } from '@playwright/test';

test.beforeEach(async ({ page }) => {
  await page.addInitScript(() => localStorage.setItem('pingo-theme', 'light'));
});

test('messages send immediately, edit, react, delete, and safely render markdown', async ({ page }) => {
  const errors: string[] = [];
  page.on('pageerror', (e) => errors.push(e.message));
  await page.goto('/');
  await expect(page.getByRole('heading', { name: 'design-studio', exact: true })).toBeVisible();
  const composer = page.getByRole('textbox', { name: 'Message design-studio' });
  await composer.fill('Hello from a browser :wave:');
  await page.getByRole('button', { name: 'Send message', exact: true }).click();
  const message = page.getByRole('article').filter({ hasText: 'Hello from a browser 👋' });
  await expect(message).toBeVisible();
  await message.hover();
  await message.getByRole('button', { name: 'Edit message', exact: true }).click();
  await page.getByRole('textbox', { name: 'Edit message' }).fill('A little **better**.');
  await page.getByRole('button', { name: 'Save changes' }).click();
  await expect(message).toHaveCount(0);
  const edited = page.getByRole('article').filter({ hasText: 'A little better.' });
  await expect(edited.locator('strong').filter({ hasText: 'better' })).toBeVisible();
  await edited.hover();
  await edited.getByRole('button', { name: 'React to message' }).click();
  await edited.getByRole('button', { name: 'React 💜', exact: true }).click();
  await expect(edited.getByRole('button', { name: '💜, 1 reactions' })).toHaveAttribute(
    'aria-pressed',
    'true',
  );
  await edited.getByRole('button', { name: '💜, 1 reactions' }).click();
  await expect(edited.locator('.reactions')).toHaveCount(0);
  await edited.hover();
  await edited.getByRole('button', { name: 'Delete message', exact: true }).click();
  await edited.getByRole('button', { name: 'Delete', exact: true }).click();
  await expect(page.getByText('This message was deleted.', { exact: true })).toBeVisible();
  await composer.fill('<img src=x onerror="window.pingoUnsafe=true"> [bad](javascript:alert(1))');
  await composer.press('Enter');
  expect(
    await page.evaluate(() => (window as Window & { pingoUnsafe?: boolean }).pingoUnsafe),
  ).toBeUndefined();
  await expect(page.locator('.markdown a[href^="javascript:"]')).toHaveCount(0);
  expect(errors).toEqual([]);
});

test('thread replies, autocomplete, uploads, and member DMs work', async ({ page }) => {
  await page.goto('/');
  await page.getByRole('button', { name: 'The new direction' }).click();
  const panel = page.getByRole('complementary', { name: 'Thread', exact: true });
  await expect(panel.getByText('3 replies', { exact: true })).toBeVisible();
  await panel.getByRole('textbox', { name: 'Reply to thread' }).fill('One more good idea.');
  await panel.getByRole('button', { name: 'Send message', exact: true }).click();
  await expect(panel.getByText('One more good idea.', { exact: true })).toBeVisible();
  await expect(panel.getByText('4 replies', { exact: true })).toBeVisible();
  await page.getByRole('button', { name: 'Close thread' }).click();
  const composer = page.getByRole('textbox', { name: 'Message design-studio' });
  await composer.fill('Hi @May');
  await expect(page.getByRole('option', { name: /Maya Chen/ })).toBeVisible();
  await composer.press('Tab');
  await expect(composer).toHaveValue('Hi @Maya ');
  await composer.fill('A little :seed');
  await composer.press('Tab');
  await expect(composer).toHaveValue('A little 🌱 ');
  await page
    .locator('input[type=file]')
    .setInputFiles({
      name: 'hello.txt',
      mimeType: 'text/plain',
      buffer: Buffer.from('A little local file.'),
    });
  await expect(page.locator('.upload-previews')).toContainText('hello.txt');
  await page.getByRole('button', { name: 'Send message', exact: true }).click();
  await expect(page.getByRole('link', { name: /hello.txt/ })).toBeVisible();
  await page.getByRole('button', { name: 'Show members' }).click();
  const members = page.getByRole('complementary', { name: 'Members', exact: true });
  await members.getByRole('button', { name: /Jules Laurent/ }).click();
  await expect(page.getByRole('heading', { name: 'Jules Laurent', exact: true })).toBeVisible();
});

test('Catch up supports filters, keyboard navigation, and a calm empty state', async ({ page }) => {
  await page.goto('/catch-up');
  await expect(page.getByRole('heading', { name: 'A little catching up.' })).toBeVisible();
  await expect(page.locator('.inbox-card')).toHaveCount(4);
  await page.keyboard.press('j');
  await page.keyboard.press('e');
  await expect(page.locator('.inbox-card')).toHaveCount(3);
  await page.getByRole('tab', { name: 'Direct messages' }).click();
  await expect(page.locator('.inbox-card')).toHaveCount(1);
  await page.getByRole('tab', { name: 'All', exact: false }).click();
  await page.getByRole('button', { name: 'Mark all done' }).click();
  await expect(page.getByRole('heading', { name: 'Room to breathe.' })).toBeVisible();
});

test('palette keyboard navigation, preferences, and space creation work', async ({ page }) => {
  await page.goto('/');
  await page.keyboard.press('Control+k');
  await page.getByRole('combobox').fill('the-build');
  await page.getByRole('combobox').press('Enter');
  await expect(page.getByRole('heading', { name: 'the-build', exact: true })).toBeVisible();
  await page.keyboard.press('Control+k');
  await page.getByRole('combobox').fill('Change theme');
  await page.getByRole('combobox').press('Enter');
  await expect(page.locator('.app-root')).toHaveAttribute('data-theme', 'dark');
  await page.keyboard.press('?');
  await expect(page.getByRole('dialog', { name: 'A few little shortcuts.' })).toBeVisible();
  await page.keyboard.press('Escape');
  await expect(page.getByRole('dialog')).toHaveCount(0);
  await page.getByRole('button', { name: 'Open settings', exact: true }).click();
  await page.getByRole('button', { name: 'High contrast', exact: true }).click();
  await expect(page.locator('.app-root')).toHaveAttribute('data-theme', 'contrast');
  await page.getByRole('button', { name: /Compact/ }).click();
  await expect(page.locator('.app-root')).toHaveAttribute('data-density', 'compact');
  await page.screenshot({ path: 'screenshots/high-contrast-compact.png' });
  await page.getByRole('button', { name: 'Set accent #4b9472' }).click();
  await expect(page.locator('.app-root')).toHaveAttribute('style', /#4b9472/);
  await page.getByRole('button', { name: 'Create or join a space', exact: true }).click();
  await page.getByRole('textbox', { name: 'Space name' }).fill('Our little test space');
  await page.getByRole('button', { name: 'Make yourself at home', exact: true }).click();
  await expect(page.locator('.space-wordmark')).toContainText('Our little test space');
  await expect(page.getByRole('heading', { name: 'general', exact: true })).toBeVisible();
  await page.getByRole('button', { name: 'Create a room', exact: true }).click();
  await page.getByRole('textbox', { name: 'Room name' }).fill('a-fresh-room');
  await page.getByRole('button', { name: 'Open the door', exact: true }).click();
  await expect(page.getByRole('heading', { name: 'a-fresh-room', exact: true })).toBeVisible();
});

test('first call checks devices, records locally, then shares and annotates a preview', async ({
  page,
  context,
}) => {
  const errors: string[] = [];
  page.on('pageerror', (e) => errors.push(e.message));
  await context.grantPermissions(['microphone']);
  await page.goto('/');
  await page.locator('.room-call-banner').getByRole('button', { name: 'Hop in' }).click();
  await expect(page.getByRole('dialog', { name: 'Let’s find your good sound.' })).toBeVisible();
  await page.getByRole('button', { name: 'Keep going' }).click();
  await page.getByRole('button', { name: 'Enable microphone' }).click();
  await page.getByRole('button', { name: 'Record 5 seconds' }).click();
  await expect(page.getByRole('button', { name: 'Play back' })).toBeEnabled({ timeout: 8000 });
  await page.getByRole('button', { name: 'Play back' }).click();
  await page.getByRole('button', { name: 'Keep going' }).click();
  await page.getByRole('button', { name: 'Strong', exact: true }).click();
  await page.getByRole('switch', { name: /Studio/ }).check();
  await expect(page.getByRole('button', { name: 'Strong', exact: true })).toBeDisabled();
  await page.getByRole('button', { name: 'Sounds good' }).click();
  await expect(page.locator('.stage-tile')).toHaveCount(4);
  await page.getByRole('button', { name: 'Share your screen', exact: true }).click();
  await expect(page.getByRole('application', { name: /Shared screen/ })).toBeVisible();
  await page.getByRole('button', { name: 'Motion', exact: true }).click();
  await expect(page.getByRole('button', { name: 'Motion', exact: true })).toHaveClass(/active/);
  await page.getByRole('button', { name: 'Draw on screen', exact: true }).click();
  const canvas = page.locator('.shared-screen');
  const box = (await canvas.boundingBox())!;
  await page.mouse.move(box.x + 150, box.y + 100);
  await page.mouse.down();
  await page.mouse.move(box.x + 300, box.y + 200, { steps: 5 });
  await page.mouse.up();
  await expect(page.locator('.annotation-stroke').first()).toBeVisible();
  await page.getByRole('button', { name: 'Raise hand', exact: true }).click();
  await expect(page.getByRole('button', { name: 'Raise hand', exact: true })).toHaveAttribute(
    'aria-pressed',
    'true',
  );
  await page.getByRole('button', { name: 'Send 💜 in call' }).click();
  await expect(page.locator('.floating-reaction')).toBeVisible();
  await page.getByRole('button', { name: 'Keep browsing', exact: true }).click();
  await page.getByRole('button', { name: 'Catch up', exact: false }).click();
  await expect(page.getByRole('region', { name: 'Call controls' })).toBeVisible();
  await page.getByRole('button', { name: 'Mute microphone', exact: true }).click();
  await expect(page.getByRole('button', { name: 'Unmute microphone', exact: true })).toBeVisible();
  const pill = page.locator('.call-pill'),
    start = (await pill.boundingBox())!;
  await page.getByRole('button', { name: /Move call controls/ }).focus();
  await page.keyboard.press('ArrowLeft');
  expect((await pill.boundingBox())!.x).toBeLessThan(start.x);
  await page.getByRole('button', { name: 'Leave call', exact: true }).click();
  await expect(pill).toHaveCount(0);
  expect(errors).toEqual([]);
});

test('lounge movement updates spatial mix and message virtualization stays bounded', async ({ page }) => {
  await page.addInitScript(() => localStorage.setItem('pingo-sound-checked', 'true'));
  await page.goto('/s/fieldwork/r/lounge');
  await page.getByRole('button', { name: 'Come on in', exact: true }).click();
  const before = await page.locator('.lounge-person small').first().textContent();
  await page.getByRole('button', { name: /Your position/ }).focus();
  for (let i = 0; i < 6; i++) await page.keyboard.press('ArrowLeft');
  await expect(page.locator('.lounge-person small').first()).not.toHaveText(before!);
  await page.getByRole('button', { name: 'design-studio', exact: false }).first().click();
  const log = page.getByRole('log', { name: /Messages/ });
  await log.evaluate((el) => (el.scrollTop = 0));
  for (let i = 0; i < 4; i++) {
    await page.getByRole('button', { name: 'Earlier conversations' }).click();
    await log.evaluate((el) => (el.scrollTop = 0));
  }
  expect(await page.locator('.virtual-row').count()).toBeLessThan(35);
  await page.getByRole('button', { name: 'Back to the present' }).click();
  await expect(page.getByRole('button', { name: 'Back to the present' })).toHaveCount(0);
  await expect(page.getByRole('region', { name: 'Call controls' })).toBeVisible();
});

test('invite preview, password signup, magic link, OAuth demo, and expired invites', async ({ page }) => {
  await page.goto('/invite/fieldwork-friends');
  await expect(page.getByRole('heading', { name: 'Pull up a chair.' })).toBeVisible();
  await page.getByRole('button', { name: 'Join Fieldwork', exact: true }).click();
  await expect(page.getByRole('heading', { name: 'design-studio', exact: true })).toBeVisible();
  await page.getByRole('button', { name: 'Open settings', exact: true }).click();
  await page.getByRole('button', { name: 'Sign out', exact: true }).click();
  await page.getByRole('button', { name: 'Create an account', exact: true }).click();
  await page.getByRole('textbox', { name: 'Your name', exact: true }).fill('Robin Maker');
  await page.getByRole('textbox', { name: 'Email address', exact: true }).fill('robin@example.com');
  await page.getByLabel('Password', { exact: true }).fill('a-demo-password');
  await page.getByRole('button', { name: 'Make yourself at home', exact: true }).click();
  await expect(page.getByRole('heading', { name: 'design-studio', exact: true })).toBeVisible();
  await page.getByRole('button', { name: 'Open settings', exact: true }).click();
  await page.getByRole('button', { name: 'Sign out', exact: true }).click();
  await page.getByRole('button', { name: 'Email me a magic link instead' }).click();
  await page.getByRole('textbox', { name: 'Email address', exact: true }).fill('robin@example.com');
  await page.getByRole('button', { name: 'Send me a sign-in link' }).click();
  await expect(page.getByRole('heading', { name: 'A little note for you.' })).toBeVisible();
  await page.getByRole('button', { name: 'Try the demo sign-in link' }).click();
  await page.goto('/signin');
  await page.getByRole('button', { name: 'Google', exact: true }).click();
  await expect(page.getByRole('heading', { name: 'design-studio', exact: true })).toBeVisible();
  await page.goto('/invite/expired');
  await expect(page.getByRole('heading', { name: 'This little door has closed.' })).toBeVisible();
});

for (const theme of ['light', 'dark'])
  test(`final ${theme} screenshots at 1440 × 900`, async ({ page }) => {
    const errors: string[] = [];
    page.on('pageerror', (e) => errors.push(e.message));
    await page.clock.setFixedTime(new Date('2026-10-23T10:30:00Z'));
    await page.addInitScript((t) => {
      localStorage.setItem('pingo-theme', t);
      localStorage.setItem('pingo-sound-checked', 'true');
    }, theme);
    await page.goto('/');
    await page.evaluate(() => document.fonts.ready);
    await expect(page.locator('.image-attachment img')).toBeVisible();
    await expect(page.getByText('New since you were here')).toBeInViewport();
    await page.screenshot({ path: `screenshots/chat-${theme}.png`, animations: 'disabled' });
    await page.getByRole('button', { name: 'The new direction' }).click();
    await expect(page.getByRole('complementary', { name: 'Thread', exact: true })).toBeVisible();
    await page.screenshot({ path: `screenshots/thread-${theme}.png`, animations: 'disabled' });
    await page.getByRole('button', { name: 'Close thread' }).click();
    await page.locator('.room-call-banner').getByRole('button', { name: 'Hop in' }).click();
    await expect(page.locator('.stage-tile')).toHaveCount(4);
    await page.screenshot({ path: `screenshots/stage-${theme}.png`, animations: 'disabled' });
    await page.getByRole('button', { name: 'Share your screen', exact: true }).click();
    await page.locator('.shared-screen').focus();
    await page.keyboard.press('Enter');
    await page.keyboard.press('d');
    await page.screenshot({ path: `screenshots/share-${theme}.png`, animations: 'disabled' });
    await page.getByRole('button', { name: 'Keep browsing', exact: true }).click();
    await page.getByRole('button', { name: 'Catch up', exact: false }).click();
    await page.screenshot({ path: `screenshots/inbox-${theme}.png`, animations: 'disabled' });
    await page.getByRole('button', { name: /The greenhouse/ }).click();
    await page.screenshot({ path: `screenshots/lounge-${theme}.png`, animations: 'disabled' });
    await page.getByRole('button', { name: 'Open settings', exact: true }).click();
    await page.screenshot({ path: `screenshots/appearance-${theme}.png`, animations: 'disabled' });
    await page.getByRole('button', { name: 'Audio & video', exact: true }).click();
    await page.getByRole('button', { name: 'Sound check', exact: true }).click();
    await page.screenshot({ path: `screenshots/sound-check-${theme}.png`, animations: 'disabled' });
    await page.keyboard.press('Escape');
    await page.keyboard.press('Control+k');
    await page.screenshot({ path: `screenshots/palette-${theme}.png`, animations: 'disabled' });
    await page.keyboard.press('Escape');
    await page.getByRole('button', { name: 'Leave call', exact: true }).click();
    await page.getByRole('button', { name: 'Sign out', exact: true }).click();
    await page.screenshot({ path: `screenshots/signin-${theme}.png`, animations: 'disabled' });
    await page.goto('/invite/fieldwork-friends');
    await page.getByRole('button', { name: 'Google', exact: true }).click();
    await page.goto('/invite/fieldwork-friends');
    await expect(page.getByRole('button', { name: 'Join Fieldwork', exact: true })).toBeVisible();
    await page.screenshot({ path: `screenshots/invite-${theme}.png`, animations: 'disabled' });
    expect(errors).toEqual([]);
  });

test('mobile and reduced-motion layout stays usable without horizontal overflow', async ({ page }) => {
  await page.setViewportSize({ width: 390, height: 844 });
  await page.goto('/');
  await page.getByRole('button', { name: 'Open rooms', exact: true }).click();
  await page.getByRole('button', { name: 'Catch up', exact: false }).click();
  await expect(page.getByRole('heading', { name: 'A little catching up.' })).toBeVisible();
  expect(await page.evaluate(() => document.documentElement.scrollWidth)).toBe(390);
  await page.screenshot({ path: 'screenshots/mobile-light.png' });
});
