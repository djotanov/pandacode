import { expect, test, type Page } from '@playwright/test';

/* Another computer's projects and tasks, shown here (remote/browse.rs; the preview mocks
 * `remote_browse` in src/preview.ts): Open project lists your other computers, a shown
 * computer's tasks open like any mirror, and a New task in one of its projects starts there,
 * by project, with no GitHub check of a local checkout. */

async function signIn(page: Page) {
  test.slow();
  await page.goto('/');
  await page.locator('#navigation .task-item').first().waitFor();
  await page.getByRole('button', { name: 'Settings', exact: true }).click();
  await page.getByRole('tab', { name: 'Connections', exact: true }).click(); await page.getByRole('tab', { name: 'Remote access' }).click();
  const panel = page.getByRole('tabpanel', { name: 'Remote access' });
  await panel.getByRole('button', { name: 'Sign in with GitHub' }).click();
  await expect(panel.getByRole('button', { name: 'Sign out' })).toBeVisible();
  await page.getByRole('button', { name: /^back/i }).click();
}
async function showNas(page: Page) {
  await page.getByRole('button', { name: 'Open project' }).click();
  const dialog = page.getByRole('dialog', { name: 'Open project' });
  const computers = dialog.getByRole('list', { name: 'On another computer' });
  await expect(computers.getByRole('listitem')).toHaveCount(3);
  // Offline computers are listed, but can't be shown until they are back.
  await expect(computers.getByRole('button', { name: 'Show projects on Old laptop' })).toBeDisabled();
  await computers.getByRole('button', { name: 'Show projects on Home NAS' }).click();
  await expect(dialog).toBeHidden();
  await expect(page.getByText('Showing 2 projects from Home NAS')).toBeVisible();
}
const remoteTask = (page: Page) => page.locator('#navigation .task-item').filter({ hasText: 'Fix the flaky upload test' });

test('another computer’s projects and tasks show here, and its tasks open like any other', async ({ page }) => {
  await signIn(page);
  await showNas(page);
  expect(await page.evaluate(() => (window as any).__remoteBrowses)).toEqual([{ deviceId: 'dev-nas', show: true }]);

  // Its task is in the sidebar, marked as running on that computer.
  await expect(remoteTask(page).locator('.task-remote')).toHaveAttribute('aria-label', 'Runs on Home NAS');
  await remoteTask(page).click();
  await expect(page.locator('#transcript')).toContainText("raced the upload's progress callback");
  await expect(page.locator('#composer-area .composer-computer-fixed')).toHaveText('on Home NAS');

  // Its projects are in the filter, located on that computer, not in a folder here.
  await page.getByRole('button', { name: 'Filter projects' }).click();
  const atlas = page.getByRole('menuitemradio', { name: /atlas/ });
  await expect(atlas).toContainText('On Home NAS');
  await expect(page.getByRole('menuitemradio', { name: /field-notes/ })).toContainText('On Home NAS');
});

test('a New task in another computer’s project starts there by project', async ({ page }) => {
  await signIn(page);
  await showNas(page);
  await page.getByRole('button', { name: 'Filter projects' }).click();
  await page.getByRole('menuitemradio', { name: /atlas/ }).click();
  await page.locator('.new-task-button').click();
  // The computer is the project's, fixed: there is no Run on menu to change it.
  const chip = page.locator('#composer-area .composer-computer-fixed');
  await expect(chip).toHaveText('on Home NAS');
  await expect(chip).toHaveAttribute('data-tip', 'This project is on Home NAS, so its tasks run there.');
  await expect(page.locator('#composer-computer')).toHaveCount(0);
  // Nothing of it is on this disk: the inspector says where it is instead of reading a folder.
  await page.getByRole('textbox', { name: 'Message your coding agent' }).fill('Add a retry to the upload client');
  await page.getByRole('textbox', { name: 'Message your coding agent' }).press('Enter');
  await expect.poll(() => page.evaluate(() => (window as any).__remoteStarts?.length ?? 0)).toBe(1);
  const started = await page.evaluate(() => (window as any).__remoteStarts[0]);
  expect(started).toMatchObject({ deviceId: 'dev-nas', projectId: 'remote-dev-nas-atlas', prompt: 'Add a retry to the upload client' });
  // A remote project has no local checkout to compare with GitHub.
  expect(await page.evaluate(() => (window as any).__remoteRepoChecks ?? [])).toEqual([]);
});

test('Stop showing a computer removes its projects and task copies here', async ({ page }) => {
  await signIn(page);
  await showNas(page);
  await expect(remoteTask(page)).toHaveCount(1);
  await page.getByRole('button', { name: 'Open project' }).click();
  const dialog = page.getByRole('dialog', { name: 'Open project' });
  await dialog.getByRole('button', { name: 'Stop showing Home NAS' }).click();
  await expect(page.getByText('No longer showing Home NAS')).toBeVisible();
  await expect(remoteTask(page)).toHaveCount(0);
  // The dialog comes back with the computer offered again.
  await expect(page.getByRole('dialog', { name: 'Open project' }).getByRole('button', { name: 'Show projects on Home NAS' })).toBeEnabled();
  await page.keyboard.press('Escape');
  await page.getByRole('button', { name: 'Filter projects' }).click();
  await expect(page.getByRole('menuitemradio', { name: /atlas/ })).toHaveCount(0);
});

test('a computer that can’t be reached is reported, and nothing is shown', async ({ page }) => {
  await signIn(page);
  await page.evaluate(() => { (window as any).__remoteBrowseFail = 'That computer is offline.'; });
  await page.getByRole('button', { name: 'Open project' }).click();
  const dialog = page.getByRole('dialog', { name: 'Open project' });
  await dialog.getByRole('button', { name: 'Show projects on Home NAS' }).click();
  await expect(page.getByText('That computer is offline.')).toBeVisible();
  await expect(dialog).toBeVisible();
  await expect(remoteTask(page)).toHaveCount(0);
});
