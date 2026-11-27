// A person on a phone (390x844, touch): signs in, opens and closes the drawer, sends a message and
// reads the streamed answer, turns on a tool server, approves the tool call on its card, and
// closes a dialog the way Android's back does. scripts/e2e.sh prepares the account, the fake
// model connection, and the tool server, and passes them in E2E_* variables.
//
// Android's back gesture sends the open modal <dialog> a close request; desktop Chrome sends the
// same close request for Escape, which is what the headless browser can press.

import { expect, type Page, test } from "@playwright/test";

const username = process.env.E2E_USERNAME ?? "";
const password = process.env.E2E_PASSWORD ?? "";

test.beforeAll(() => {
  if (!process.env.E2E_BASE_URL || !username || !password) {
    throw new Error("Run through scripts/e2e.sh, which sets E2E_BASE_URL and the account");
  }
});

/** The page never scrolls sideways at phone width. */
async function fitsTheScreen(page: Page) {
  const overflow = await page.evaluate(
    () => document.documentElement.scrollWidth - document.documentElement.clientWidth
  );
  expect(overflow).toBeLessThanOrEqual(0);
}

/** The visible drawer button (the shell's, or a page's own top bar). */
function menuButton(page: Page) {
  return page.getByRole("button", { name: "Open menu" }).filter({ visible: true }).first();
}

async function signIn(page: Page) {
  await page.goto("/");
  await expect(page).toHaveURL(/\/login/);
  await page.getByRole("textbox", { name: "Username" }).fill(username);
  await page.getByRole("textbox", { name: "Password" }).fill(password);
  await page.getByRole("button", { name: "Sign in" }).tap();
  await expect(page.getByRole("heading", { name: "New Chat" })).toBeVisible();
}

test("a phone signs in, chats, approves a tool call, and closes dialogs with back", async ({
  page,
}) => {
  await signIn(page);
  await expect(page.locator('meta[name="viewport"]')).toHaveAttribute(
    "content",
    /interactive-widget=resizes-content/
  );
  expect(await page.evaluate(() => matchMedia("(pointer: coarse)").matches)).toBe(true);
  await fitsTheScreen(page);

  // The sidebar is a drawer on a phone: closed until the menu button opens it, and a close
  // request (Android's back) closes it again.
  const drawer = page.locator("dialog#drawer");
  await expect(drawer).not.toBeVisible();
  await menuButton(page).tap();
  await expect(drawer).toBeVisible();
  await expect(drawer.getByRole("link", { name: "New chat" })).toBeVisible();
  await page.keyboard.press("Escape");
  await expect(drawer).not.toBeVisible();

  // On a touch screen Enter adds a line; the Send button sends.
  const composer = page.getByPlaceholder("Message");
  await composer.tap();
  await composer.pressSequentially("hello");
  await composer.press("Enter");
  await composer.pressSequentially("from a phone");
  await expect(composer).toHaveValue("hello\nfrom a phone");
  await page.getByRole("button", { name: "Send" }).tap();
  await expect(page).toHaveURL(/\/chats\/[0-9a-f-]+$/);
  await expect(page.getByText("You said: hello from a phone")).toBeVisible();
  await fitsTheScreen(page);

  // The tool server goes on for this chat in the composer's tools menu; the call waits on its
  // approval card until the person taps Approve.
  await page.getByRole("button", { name: "Tools" }).tap();
  // The chat stores the choice, and the box shows it once the server has it.
  const tools = page.getByRole("checkbox", { name: "Fake tools" });
  await tools.tap();
  await expect(tools).toBeChecked();
  await page.getByRole("button", { name: "Tools" }).tap();
  await composer.tap();
  await composer.fill('tool echo {"text": "from the phone"}');
  await page.getByRole("button", { name: "Send" }).tap();
  const approvals = page.getByRole("region", { name: "Tool approvals" });
  await expect(approvals).toBeVisible();
  await expect(page).toHaveTitle(/^\(!\)/);
  await approvals.getByRole("button", { name: "Approve", exact: true }).tap();
  await expect(page.getByText("Tool said: from the phone")).toBeVisible();
  await expect(approvals).not.toBeVisible();
  await fitsTheScreen(page);

  // Back closes the drawer over a chat too, and leaves the chat where it was.
  await menuButton(page).tap();
  await expect(drawer).toBeVisible();
  await page.keyboard.press("Escape");
  await expect(drawer).not.toBeVisible();
  await expect(page).toHaveURL(/\/chats\/[0-9a-f-]+$/);
  await expect(page.getByText("Tool said: from the phone")).toBeVisible();

  // The chat is in the drawer's list, and following it closes the drawer.
  await menuButton(page).tap();
  await drawer.getByRole("link", { name: "New chat" }).tap();
  await expect(drawer).not.toBeVisible();
  await expect(page.getByRole("heading", { name: "New Chat" })).toBeVisible();
});
