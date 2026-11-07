import { expect, test, type Page } from '@playwright/test';

// Captures every reachable screen in mock mode for design review. Steps are best-effort: a screen that
// fails to open is logged and skipped so one regression doesn't hide the rest of the gallery.
const dir = 'screenshots/review';
test.use({ actionTimeout: 5000 });

async function snap(page: Page, name: string, open: () => Promise<unknown>) {
  try {
    await open();
    await page.waitForTimeout(150);
    await page.screenshot({ path: `${dir}/${name}.png`, animations: 'disabled' });
  } catch (e) {
    console.warn(`gallery: skipped ${name}: ${(e as Error).message.split('\n')[0]}`);
  }
}

async function boot(page: Page, theme: string, extra: Record<string, string> = {}) {
  await page.clock.setFixedTime(new Date('2026-10-23T10:30:00Z'));
  await page.addInitScript(
    ([t, e]) => {
      localStorage.setItem('pingo-theme', t as string);
      localStorage.setItem('pingo-sound-checked', 'true');
      for (const [k, v] of Object.entries(e as Record<string, string>)) localStorage.setItem(k, v);
    },
    [theme, extra] as const,
  );
}

for (const theme of ['light', 'dark'])
  test(`gallery desktop ${theme}`, async ({ page, context }) => {
    test.setTimeout(180_000);
    await context.grantPermissions(['microphone', 'camera']);
    await boot(page, theme);
    await page.goto('/');
    await page.evaluate(() => document.fonts.ready);
    const t = (n: string) => `${theme}-${n}`;
    await snap(page, t('chat'), () => expect(page.locator('.image-attachment img')).toBeVisible());
    await snap(page, t('message-hover'), () => page.getByRole('article').last().hover());
    await snap(page, t('composer-autocomplete'), () =>
      page.getByRole('textbox', { name: 'Message design-studio' }).fill('Hi @'),
    );
    await page.getByRole('textbox', { name: 'Message design-studio' }).fill('');
    await snap(page, t('workspace-menu'), () => page.locator('.space-heading-main').click());
    await page.keyboard.press('Escape');
    if (await page.locator('.workspace-menu').count()) await page.locator('.space-heading-main').click();
    await snap(page, t('thread'), () => page.getByRole('button', { name: 'The new direction' }).click());
    await page.getByRole('button', { name: 'Close thread' }).click();
    await snap(page, t('members'), () => page.getByRole('button', { name: 'Show members' }).click());
    await snap(page, t('dm'), () =>
      page
        .getByRole('complementary', { name: 'Members', exact: true })
        .getByRole('button', { name: /Jules Laurent/ })
        .click(),
    );
    await page.goto('/s/fieldwork/r/build');
    await snap(page, t('room-build'), () =>
      expect(page.getByRole('heading', { name: 'the-build', exact: true })).toBeVisible(),
    );
    await snap(page, t('room-settings'), () => page.getByRole('button', { name: 'Room settings' }).click());
    await page.keyboard.press('Escape');
    await snap(page, t('room-empty'), () => page.goto('/s/fieldwork/r/wins'));
    await snap(page, t('stage-room'), () => page.goto('/s/fieldwork/r/stage'));
    await snap(page, t('lounge-preview'), () => page.goto('/s/fieldwork/r/lounge'));
    await snap(page, t('lounge-joined'), () =>
      page.getByRole('button', { name: 'Join lounge', exact: true }).click(),
    );
    await page
      .getByRole('button', { name: 'Leave call', exact: true })
      .click()
      .catch(() => {});
    await page.goto('/');
    await snap(page, t('create-space'), () =>
      page.getByRole('button', { name: 'Create or join a space', exact: true }).click(),
    );
    await page.keyboard.press('Escape');
    await snap(page, t('create-room'), () =>
      page.getByRole('button', { name: 'Create a room', exact: true }).click(),
    );
    await page.keyboard.press('Escape');
    await snap(page, t('status'), () => page.locator('.self-status').click());
    await page.keyboard.press('Escape');
    await snap(page, t('shortcuts'), () => page.keyboard.press('?'));
    await page.keyboard.press('Escape');
    await snap(page, t('palette'), () => page.keyboard.press('Control+k'));
    await snap(page, t('palette-search'), () => page.getByRole('combobox').fill('launch'));
    await page.keyboard.press('Escape');
    await snap(page, t('inbox'), () => page.getByRole('button', { name: 'Inbox', exact: true }).click());
    await snap(page, t('inbox-empty'), () => page.getByRole('button', { name: 'Mark all done' }).click());
    await page.goto('/');
    await snap(page, t('call-stage'), async () => {
      await page.locator('.room-call-banner').getByRole('button', { name: 'Join', exact: true }).click();
      await expect(page.locator('.stage-tile')).toHaveCount(4);
    });
    await snap(page, t('call-share'), () =>
      page.getByRole('button', { name: 'Share your screen', exact: true }).click(),
    );
    await snap(page, t('call-pill'), async () => {
      await page.getByRole('button', { name: 'Minimize call', exact: true }).click();
      await page.getByRole('button', { name: 'Inbox', exact: true }).click();
    });
    for (const tab of [
      'Profile',
      'Audio & video',
      'Appearance',
      'Notifications',
      'Keyboard shortcuts',
      'Space',
    ])
      await snap(page, t(`settings-${tab.toLowerCase().replace(/[^a-z]+/g, '-')}`), async () => {
        const nav = page.getByRole('navigation', { name: 'Settings sections' });
        if (!(await nav.count()))
          await page.getByRole('button', { name: 'Open settings', exact: true }).click();
        await nav.getByRole('button', { name: tab, exact: true }).click();
      });
    await page.getByRole('button', { name: 'Audio & video', exact: true }).click();
    await snap(page, t('sound-check-1'), () =>
      page.getByRole('button', { name: 'Sound check', exact: true }).click(),
    );
    await snap(page, t('sound-check-2'), () =>
      page.getByRole('button', { name: 'Next', exact: true }).click(),
    );
    await snap(page, t('sound-check-3'), () =>
      page.getByRole('button', { name: 'Enable microphone' }).click(),
    );
    await snap(page, t('sound-check-4'), () =>
      page.getByRole('button', { name: 'Next', exact: true }).click(),
    );
    await page.keyboard.press('Escape');
    await page
      .getByRole('button', { name: 'Leave call', exact: true })
      .click()
      .catch(() => {});
    await page
      .getByRole('button', { name: 'Open settings', exact: true })
      .click()
      .catch(() => {});
    await snap(page, t('signin'), () => page.getByRole('button', { name: 'Sign out', exact: true }).click());
    await snap(page, t('signup'), () =>
      page.getByRole('button', { name: 'Create an account', exact: true }).click(),
    );
    await page.goto('/signin');
    await snap(page, t('magic-link'), () =>
      page.getByRole('button', { name: 'Email me a magic link instead' }).click(),
    );
    await snap(page, t('magic-link-sent'), async () => {
      await page.getByRole('textbox', { name: 'Email address', exact: true }).fill('robin@example.com');
      await page.getByRole('button', { name: 'Send me a sign-in link' }).click();
    });
    await page.goto('/signin');
    await page.getByRole('button', { name: 'Google', exact: true }).click();
    await snap(page, t('invite'), () => page.goto('/invite/fieldwork-friends'));
    await snap(page, t('invite-expired'), () => page.goto('/invite/expired'));
  });

test('gallery contrast compact', async ({ page }) => {
  await boot(page, 'contrast', { 'pingo-density': 'compact' });
  await page.goto('/');
  await snap(page, 'contrast-compact-chat', () =>
    expect(page.locator('.image-attachment img')).toBeVisible(),
  );
  await snap(page, 'contrast-compact-thread', () =>
    page.getByRole('button', { name: 'The new direction' }).click(),
  );
});

for (const theme of ['light', 'dark'])
  test(`gallery mobile ${theme}`, async ({ page }) => {
    await page.setViewportSize({ width: 390, height: 844 });
    await boot(page, theme);
    await page.goto('/');
    const t = (n: string) => `mobile-${theme}-${n}`;
    await snap(page, t('chat'), () => expect(page.locator('.image-attachment img')).toBeVisible());
    await snap(page, t('sidebar'), () =>
      page.getByRole('button', { name: 'Open rooms', exact: true }).click(),
    );
    await page
      .getByRole('button', { name: 'Close sidebar' })
      .click()
      .catch(() => {});
    await snap(page, t('thread'), () => page.getByRole('button', { name: 'The new direction' }).click());
    await page
      .getByRole('button', { name: 'Close thread' })
      .click()
      .catch(() => {});
    await snap(page, t('call'), async () => {
      await page.locator('.room-call-banner').getByRole('button', { name: 'Join', exact: true }).click();
      await expect(page.locator('.stage-tile').first()).toBeVisible();
    });
    await page
      .getByRole('button', { name: 'Minimize call', exact: true })
      .click()
      .catch(() => {});
    await snap(page, t('inbox'), async () => {
      await page.getByRole('button', { name: 'Open rooms', exact: true }).click();
      await page.getByRole('button', { name: 'Inbox', exact: true }).click();
    });
    await snap(page, t('settings'), async () => {
      await page.getByRole('button', { name: 'Open rooms', exact: true }).click();
      await page.getByRole('button', { name: 'Open settings', exact: true }).click();
    });
    await snap(page, t('palette'), () => page.keyboard.press('Control+k'));
    await page.keyboard.press('Escape');
    await page.goto('/signin');
    await snap(page, t('signin'), async () => {
      if (!(await page.getByRole('heading', { name: 'Sign in' }).count())) {
        await page.goto('/settings');
        await page.getByRole('button', { name: 'Sign out', exact: true }).click();
      }
    });
  });
