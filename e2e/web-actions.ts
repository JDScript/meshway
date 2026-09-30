import { expect, type Page } from "@playwright/test";

export async function selectNode(page: Page, testId: string, name: RegExp) {
  const trigger = page.getByTestId(testId);
  await trigger.click();
  await expect(trigger).toHaveAttribute("aria-expanded", "true");
  const popupId = await trigger.getAttribute("aria-controls");
  if (!popupId) throw new Error(`Select ${testId} did not identify its listbox`);
  await page.locator(`[id=${JSON.stringify(popupId)}]`).getByRole("option", { name }).click();
  await expect(trigger).toHaveAttribute("aria-expanded", "false");
}

export async function editRuleId(page: Page, current: string, next: string) {
  await page.getByRole("button", { name: `编辑规则 ${current}`, exact: true }).click();
  const editor = page.getByRole("dialog");
  await editor.getByLabel("规则 ID", { exact: true }).fill(next);
  await page.keyboard.press("ControlOrMeta+k");
  await expect(page.getByRole("dialog", { name: "搜索与跳转", exact: true })).not.toBeVisible();
  await expect(editor.getByLabel("规则 ID", { exact: true })).toHaveValue(next);
  await editor.getByRole("button", { name: "应用到草稿", exact: true }).click();
  await expect(editor).not.toBeVisible();
  await expect(page.getByTestId("policy-rule").filter({ hasText: next })).toBeVisible();
}

export async function discardPolicyDraft(page: Page) {
  await page.getByRole("button", { name: "放弃修改", exact: true }).click();
  const confirmation = page.getByRole("alertdialog");
  await confirmation.getByRole("button", { name: "放弃修改", exact: true }).click();
  await expect(confirmation).not.toBeVisible();
}
