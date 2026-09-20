// All API traffic is intercepted. Each test gets a fresh browser context and
// mutable fixture; no real accounts, memberships or project data are used.
import { test as base, expect } from '@playwright/test';

const uid = '01ARZ3NDEKTSV4RRFFQ69G5FAV';
const gid = '01ARZ3NDEKTSV4RRFFQ69G5FAW';
const pid = '01ARZ3NDEKTSV4RRFFQ69G5FAX';
const alice = '01ARZ3NDEKTSV4RRFFQ69G5FAZ';
const bob = '01ARZ3NDEKTSV4RRFFQ69G5FB0';
const otherGroup = '01ARZ3NDEKTSV4RRFFQ69G5FB1';
const charlie = '01ARZ3NDEKTSV4RRFFQ69G5FB2';
const now = '2026-09-20T09:00:00Z';
const me = { id: uid, provider: 'local', subject: 'ui-test', handle: 'owner', name: 'Test Owner' };
const member = (id, handle, name, owner = false) => ({
  user_id: id, handle, name, owner, invited_by: uid, since: now,
});
const group = (id, name) => ({ id, name, description: null, kind: 'shared', owner: uid, created_at: now });
const project = { id: pid, group_id: gid, name: 'Test project', description: null, repository: null, created_at: now };
const json = (status, data) => ({ status, contentType: 'application/json', body: JSON.stringify(data) });
const failure = json(500, { error: { code: 'internal', message: 'internal error' } });
const pageOf = items => ({ items, next_cursor: null });
const button = (page, name) => page.getByRole('button', { name, exact: true });
const toast = (page, text) => page.locator('.cv-toast').filter({ hasText: text });
const settings = '#/group/settings';

const test = base.extend({
  app: async ({ page }, use) => {
    const state = {
      roster: [member(uid, 'owner', 'Test Owner', true), member(alice, 'alice', 'Alice'), member(bob, 'bob', 'Bob')],
      groups: [group(gid, 'Test group')],
    };
    const plans = [];
    const releases = [];
    const requests = [];
    const exceptions = [];
    const diagnostics = [];
    page.on('pageerror', error => exceptions.push(error.message));
    await page.exposeFunction('recordUiConsole', (method, args) => diagnostics.push({ method, args }));
    await page.addInitScript(() => {
      for (const method of ['log', 'info', 'debug', 'warn', 'error', 'trace', 'dir', 'table']) {
        console[method] = (...args) => { void window.recordUiConsole(method, args.map(String)); };
      }
    });
    const defer = (method, suffix, response) => {
      let release, received;
      const wait = new Promise(resolve => { release = resolve; });
      const sent = new Promise(resolve => { received = resolve; });
      plans.push({ method, suffix, response, wait, received });
      releases.push(release);
      return { sent, release };
    };
    await page.route('**/api/v1/**', async route => {
      const request = route.request();
      const path = new URL(request.url()).pathname;
      const method = request.method();
      requests.push({ method, path });
      const index = plans.findIndex(plan => plan.method === method && path.endsWith(plan.suffix));
      if (index >= 0) {
        const plan = plans.splice(index, 1)[0];
        plan.received();
        await plan.wait;
        return route.fulfill(plan.response);
      }
      const reply = data => route.fulfill(json(200, data));
      if (path.endsWith('/me')) return reply(me);
      if (path.endsWith('/auth')) return reply({ oidc: null });
      if (path.endsWith('/groups') && method === 'GET') return reply(pageOf(state.groups));
      if (path.endsWith('/users')) return reply(pageOf([me]));
      if (path.endsWith('/projects') && method === 'GET') return reply(pageOf([project]));
      if (path.endsWith('/members')) {
        if (method === 'GET') return reply(state.roster);
        state.roster.push(member(charlie, 'charlie', 'Charlie'));
        return route.fulfill({ status: 204 });
      }
      if (path.endsWith('/members/' + bob) && method === 'DELETE') {
        state.roster = state.roster.filter(member => member.user_id !== bob);
        return route.fulfill({ status: 204 });
      }
      if (path.endsWith('/groups/' + gid) && method === 'DELETE') return route.fulfill({ status: 204 });
      return method === 'GET' ? reply(pageOf([])) : route.fulfill(failure);
    });
    const open = async (hash = settings) => {
      await page.goto('/' + hash);
      await expect(page.getByRole('navigation', { name: 'Main navigation' })).toBeVisible();
      if (hash === settings) await expect(button(page, 'Remove Alice')).toBeVisible();
    };
    const invite = async () => {
      await page.getByRole('button', { name: /Invite$/ }).click();
      await page.getByRole('textbox', { name: 'Converge handle' }).fill('charlie');
      await button(page, 'Add').click();
      await expect(page.locator('.cv-modal')).toHaveCount(0);
      await expect(button(page, 'Remove Charlie')).toBeVisible();
    };
    const startDelete = async kind => {
      await button(page, 'Delete').click();
      await page.locator('.cv-modal input').fill('Test ' + kind);
      const pending = defer('DELETE', '/' + kind + 's/' + (kind === 'group' ? gid : pid), { status: 204 });
      await button(page, 'Delete ' + kind).click();
      await pending.sent;
      return pending;
    };
    try {
      await use({ state, requests, defer, open, invite, startDelete });
      expect(exceptions, 'browser exceptions').toEqual([]);
      expect(diagnostics, 'application console calls').toEqual([]);
    } finally {
      releases.forEach(release => release());
      await page.unrouteAll({ behavior: 'ignoreErrors' });
    }
  },
});

test('member error survives another removal and an invitation reload', async ({ page, app }) => {
  await app.open();
  await button(page, 'Remove Alice').click();
  const error = page.locator('.cv-memberrow').filter({ hasText: '@alice' }).getByRole('alert');
  await expect(error).toContainText("Couldn't remove teammate");
  await button(page, 'Remove Bob').click();
  await expect(button(page, 'Remove Bob')).toHaveCount(0);
  await expect(error).toContainText("Couldn't remove teammate");
  // Display data can change even though row identity and its action stay alive.
  app.state.roster.find(member => member.user_id === alice).name = 'Alice Updated';
  await app.invite();
  await expect(button(page, 'Remove Alice Updated')).toBeVisible();
  await expect(error).toContainText("Couldn't remove teammate");
});

test('pending removal stays guarded through roster changes', async ({ page, app }) => {
  await app.open();
  const pending = app.defer('DELETE', '/members/' + alice, failure);
  await button(page, 'Remove Alice').click();
  await pending.sent;
  await button(page, 'Remove Bob').click();
  await expect(button(page, 'Remove Bob')).toHaveCount(0);
  await app.invite();
  await expect(button(page, 'Remove Alice')).toBeDisabled();
  await button(page, 'Remove Alice').dispatchEvent('click');
  pending.release();
  await expect(page.locator('.cv-memberrow').filter({ hasText: '@alice' }).getByRole('alert')).toContainText("Couldn't remove teammate");
  expect(app.requests.filter(request => request.method === 'DELETE' && request.path.endsWith(alice))).toHaveLength(1);
  await expect(button(page, 'Remove Alice')).toBeEnabled();
});

test('disabled input has a visible state and recovers after failure', async ({ page, app }) => {
  await app.open();
  await page.getByRole('button', { name: /Invite$/ }).click();
  const field = page.getByRole('textbox', { name: 'Converge handle' });
  await field.fill('charlie');
  await field.blur();
  const appearance = () => field.evaluate(element => ({
    color: getComputedStyle(element).color,
    background: getComputedStyle(element.parentElement).backgroundColor,
    cursor: getComputedStyle(element).cursor,
  }));
  const before = await appearance();
  const pending = app.defer('POST', '/members', failure);
  await button(page, 'Add').click();
  await pending.sent;
  await expect(field).toBeDisabled();
  const during = await appearance();
  expect(during.color).not.toBe(before.color);
  expect(during.background).not.toBe(before.background);
  expect(during.cursor).toBe('not-allowed');
  pending.release();
  await expect(field).toBeEnabled();
  await expect(field).toHaveValue('charlie');
  expect(await appearance()).toEqual(before);
});

for (const kind of ['project', 'group']) {
  test(`successful ${kind} deletion after Cancel leaves a usable screen`, async ({ page, app }) => {
    await app.open(kind === 'group' ? settings : '#/project/' + pid + '/settings');
    const pending = await app.startDelete(kind);
    await button(page, 'Cancel').click();
    pending.release();
    await expect(toast(page, 'deleted.')).toBeVisible();
    await expect(page).toHaveURL(/\/#\/$/);
    await expect(page.locator('.cv-setform')).toHaveCount(0);
    await expect(page.getByRole('alert').filter({ hasText: "Couldn't load members" })).toHaveCount(0);
    await expect(page.getByRole('button', { name: kind === 'group' ? /New group/ : /New project/ }).first()).toBeVisible();
  });
}

test('late deletion preserves navigation and a different selected group', async ({ page, app }) => {
  app.state.groups.push(group(otherGroup, 'Other group'));
  await app.open();
  const pending = await app.startDelete('group');
  await button(page, 'Cancel').click();
  await page.getByRole('button', { name: /Other group/ }).click();
  await page.evaluate(hash => { location.hash = hash; }, settings);
  await expect(page.locator('.cv-setform input').first()).toHaveValue('Other group');
  pending.release();
  await expect(toast(page, 'deleted.')).toBeVisible();
  await expect(page).toHaveURL(/#\/group\/settings$/);
  await expect(page.locator('.cv-setform input').first()).toHaveValue('Other group');
  await expect(page.locator('.cv-sidebar__groups .cv-nav--active')).toContainText('Other group');
});

test('late project deletion does not pull the user away from account settings', async ({ page, app }) => {
  await app.open('#/project/' + pid + '/settings');
  const pending = await app.startDelete('project');
  await button(page, 'Cancel').click();
  await page.evaluate(() => { location.hash = '#/settings'; });
  await expect(page.getByPlaceholder("What's this token for? (laptop, ci, …)")).toBeVisible();
  pending.release();
  await expect(toast(page, 'deleted.')).toBeVisible();
  await expect(page).toHaveURL(/#\/settings$/);
});

for (const remaining of [0, 1]) {
  test(`group deletion with ${remaining} remaining groups does not load a stale roster`, async ({ page, app }) => {
    if (remaining) app.state.groups.push(group(otherGroup, 'Other group'));
    await app.open();
    await page.evaluate(() => {
      window.memberLoadErrors = [];
      new MutationObserver(() => {
        for (const alert of document.querySelectorAll('[role=alert]')) {
          if (alert.textContent.includes("Couldn't load members")) window.memberLoadErrors.push(alert.textContent);
        }
      }).observe(document.body, { subtree: true, childList: true, characterData: true });
    });
    const requestCount = app.requests.length;
    const pending = await app.startDelete('group');
    pending.release();
    await expect(toast(page, 'deleted.')).toBeVisible();
    await expect(page).toHaveURL(/\/#\/$/);
    expect(await page.evaluate(() => window.memberLoadErrors)).toEqual([]);
    expect(app.requests.slice(requestCount).filter(request => request.path.endsWith('/members'))).toEqual([]);
  });
}

test('repeated sign-out failures share one notification with a count', async ({ page, app }) => {
  await app.open();
  for (let attempt = 1; attempt <= 5; attempt++) {
    await page.getByRole('button', { name: /Test Owner @owner/ }).click();
    await page.locator('.cv-acctmenu__item--danger').click();
    await expect(toast(page, "Couldn't sign out")).toBeVisible();
    if (attempt > 1) await expect(page.locator('.cv-toast__count')).toHaveText('×' + attempt);
  }
  await expect(page.locator('.cv-toast')).toHaveCount(1);
  await button(page, 'Dismiss').click();
  await expect(page.locator('.cv-toast')).toHaveCount(0);
});

test('independent failures stay available when folded and gaps pass clicks through', async ({ page, app }) => {
  await app.open();
  const signOut = async () => {
    await page.getByRole('button', { name: /Test Owner @owner/ }).click();
    await page.locator('.cv-acctmenu__item--danger').click();
  };
  await signOut();
  await expect(toast(page, "Couldn't sign out")).toBeVisible();
  for (let index = 1; index <= 8; index++) {
    await button(page, 'New project').click();
    await page.locator('.cv-modal input').fill('Draft ' + index);
    const pending = app.defer('POST', '/projects', failure);
    await button(page, 'Create project').click();
    await pending.sent;
    await button(page, 'Cancel').click();
    pending.release();
    await expect(page.locator('.cv-toast')).toHaveCount(Math.min(index + 1, 5));
    if (index >= 5) await expect(button(page, `Show all ${index + 1} notifications`)).toBeVisible();
  }
  await expect(page.locator('.cv-toast__count:visible')).toHaveCount(0);
  await expect(toast(page, "Couldn't sign out")).toHaveCount(0);
  await signOut();
  await expect(toast(page, "Couldn't sign out")).toBeVisible();
  await expect(toast(page, "Couldn't sign out").locator('.cv-toast__count')).toHaveText('×2');
  // Verify real hit testing and a real click in a gap, not just computed CSS.
  const gap = await page.evaluate(() => {
    const [first, second] = document.querySelectorAll('.cv-toast');
    const a = first.getBoundingClientRect(), b = second.getBoundingClientRect();
    const x = a.x + a.width / 2, y = (a.bottom + b.top) / 2;
    const target = document.createElement('button');
    target.textContent = 'Underlying control';
    const layer = Number(getComputedStyle(document.querySelector('.cv-toasts')).zIndex) - 1;
    target.style.cssText = `position:fixed;left:${x - 5}px;top:${y - 3}px;width:10px;height:6px;z-index:${layer};padding:0`;
    target.addEventListener('click', () => { window.clickedUnderlying = true; });
    document.body.append(target);
    return { x, y, passes: document.elementFromPoint(x, y) === target };
  });
  expect(gap.passes).toBe(true);
  await page.mouse.click(gap.x, gap.y);
  expect(await page.evaluate(() => window.clickedUnderlying)).toBe(true);
  await button(page, 'Show all 9 notifications').click();
  await expect(page.locator('.cv-toast')).toHaveCount(9);
  // A visible expanded panel owns scrolling; all retained notices are reachable.
  await page.setViewportSize({ width: 390, height: 844 });
  await expect.poll(() => page.locator('.cv-toasts').evaluate(element => element.scrollHeight > element.clientHeight)).toBe(true);
  await button(page, 'Dismiss').last().click();
  await expect(page.locator('.cv-toast')).toHaveCount(8);
  await button(page, 'Show fewer notifications').click();
  await expect(page.locator('.cv-toast')).toHaveCount(5);
  await button(page, 'Show all 8 notifications').click();
  await expect(page.locator('.cv-toast')).toHaveCount(8);
  while (await button(page, 'Dismiss').count()) await button(page, 'Dismiss').first().click();
  await expect(page.locator('.cv-toast')).toHaveCount(0);
  await expect(page.locator('.cv-toasts__toggle')).toHaveCount(0);
});
