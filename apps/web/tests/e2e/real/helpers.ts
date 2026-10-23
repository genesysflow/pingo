import { expect, type Page } from '@playwright/test';
export async function signup(page: Page, name: string) {
  await page.goto('/signup');
  await page.getByLabel('Your name', { exact: true }).fill(name);
  await page.getByLabel('Email address', { exact: true }).fill(`${name.toLowerCase()}-${crypto.randomUUID()}@pingo.test`);
  await page.getByLabel('Password', { exact: true }).fill('correct-horse-pingo');
  await page.getByRole('button', { name: 'Make yourself at home', exact: true }).click();
  await expect(page.getByRole('heading', { name: 'Make a little space.' })).toBeVisible();
}
export async function createSpace(page: Page) {
  await page.getByLabel('Space name', { exact: true }).fill(`Browser friends ${Date.now()}`);
  await page.getByRole('button', { name: 'Create space', exact: true }).click();
  await expect(page.getByRole('heading', { name: 'general', exact: true })).toBeVisible();
  return page.url();
}
export async function invite(page: Page) {
  await page.locator('.space-heading-main').click();
  await page.getByRole('button', { name: 'Space settings & invites' }).click();
  await page.getByRole('button', { name: 'Create an invite', exact: false }).click();
  const link = await page.getByLabel('Invite link', { exact: true }).inputValue();
  await page.getByRole('button', { name: 'Back to your people' }).click();
  return link;
}
export async function accept(page: Page, link: string) {
  await page.goto(link);
  await page.getByRole('button', { name: /^Join Browser friends/ }).click();
  await expect(page.getByRole('heading', { name: 'general', exact: true })).toBeVisible();
}
export async function send(page: Page, body: string) {
  const input = page.getByRole('textbox', { name: 'Message general', exact: true });
  await input.fill(body); await page.getByRole('button', { name: 'Send message', exact: true }).click();
  await expect(page.getByRole('article').filter({ hasText: body }).first()).toBeVisible();
}
