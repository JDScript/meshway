import { mkdir, readFile, rm, writeFile } from "node:fs/promises"
import { fileURLToPath } from "node:url"

import { codeSamples } from "../src/lib/openapi-samples.ts"
import { apiTags, getReference } from "../src/lib/openapi.ts"

const output = fileURLToPath(new URL("../content/docs/api/", import.meta.url))
const sdk = await readFile(
  new URL(
    "../../../packages/control-client/src/generated/operations.gen.ts",
    import.meta.url
  ),
  "utf8"
)
const { document, operations } = await getReference()
const titles = {
  getAccount: "读取账户",
  listAccountActivity: "列出账户活动",
  changeAccountEmail: "修改邮箱",
  changeAccountPassword: "修改密码",
  listAccountSessions: "列出会话",
  revokeAllAccountSessions: "撤销全部会话",
  revokeOtherAccountSessions: "撤销其他会话",
  revokeAccountSession: "撤销会话",
  login: "登录",
  logout: "退出登录",
  createDelegation: "签发委托登录授权码",
  exchangeDelegation: "兑换委托登录会话",
  getAuthOptions: "读取认证选项",
  inspectPasswordReset: "检查密码重置链接",
  redeemPasswordReset: "重置密码",
  register: "注册",
  getAuthSession: "读取当前会话",
  getConfigurationSchema: "读取配置 Schema",
  getDeviceAuthorization: "读取设备授权",
  approveDeviceAuthorization: "批准设备接入",
  denyDeviceAuthorization: "撤回设备批准",
  inspectJoinLink: "检查加入链接",
  redeemJoinLink: "使用加入链接",
  listNetworks: "列出网络",
  createNetwork: "创建网络",
  getNetwork: "读取网络",
  listNetworkActivity: "列出网络活动",
  getNetworkActivity: "读取网络活动",
  getConfiguration: "读取配置",
  applyConfiguration: "应用配置",
  compareConfiguration: "比较配置版本",
  evaluateConfiguration: "路径检测",
  listConfigurationRevisions: "列出配置版本",
  getConfigurationRevision: "读取配置版本",
  getConfigurationProgress: "读取配置应用进度",
  getConfigurationSource: "读取配置原文",
  validateConfiguration: "验证配置",
  listEnrollmentKeys: "列出接入密钥",
  createEnrollmentKey: "创建接入密钥",
  revokeEnrollmentKey: "撤销接入密钥",
  watchNetwork: "订阅网络事件",
  queryObservationHistory: "查询观测历史",
  listJoinLinks: "列出加入链接",
  createJoinLink: "创建加入链接",
  revokeJoinLink: "撤销加入链接",
  leaveNetwork: "离开网络",
  getLeaveImpact: "预览离网影响",
  listNetworkMembers: "列出网络成员",
  getMemberRemovalImpact: "预览移除成员影响",
  removeMember: "移除成员",
  changeMemberRole: "修改成员角色",
  getMemberRoleChangeImpact: "预览角色变更影响",
  listNodes: "列出节点",
  getNode: "读取节点",
  deleteNode: "删除节点",
  revokeNodeKey: "撤销节点密钥",
  getNodeSettings: "读取节点设置",
  getNodeState: "读取节点状态",
  listObservations: "列出观测订阅",
  createObservation: "创建观测订阅",
  getObservation: "读取观测订阅",
  cancelObservation: "取消观测订阅",
  renewObservation: "续租观测订阅",
  renameNetwork: "重命名网络",
  listRuleSets: "列出规则集",
  getRuleSet: "读取规则集",
  getRuleSetReport: "读取规则集转换报告",
  listRuleSetCatalog: "搜索规则集目录",
  listRuleSetEntries: "列出规则集成员",
  matchRuleSet: "检查规则集是否命中",
  refreshRuleSet: "刷新规则集",
  getNetworkState: "读取网络状态",
  getTransportCatalog: "读取传输目录",
}

await mkdir(output, { recursive: true })
for (const tag of Object.keys(apiTags)) {
  // Only generated tag directories are replaced; authored overview pages live at the root.
  await rm(`${output}${tag}`, { recursive: true, force: true })
  await mkdir(`${output}${tag}`, { recursive: true })
}

for (const lang of ["zh", "en"]) {
  const suffix = lang === "zh" ? "" : ".en"
  await writeFile(
    `${output}meta${suffix}.json`,
    JSON.stringify(
      {
        title: lang === "zh" ? "API 参考" : "API reference",
        description: lang === "zh" ? "用户 HTTP API" : "User HTTP API",
        root: true,
        pages: ["index", ...Object.keys(apiTags)],
      },
      null,
      2
    )
  )
  for (const [tag, names] of Object.entries(apiTags)) {
    const entries = operations.filter((entry) => entry.tag === tag)
    await writeFile(
      `${output}${tag}/meta${suffix}.json`,
      JSON.stringify(
        { title: names[lang], pages: entries.map((entry) => entry.slug) },
        null,
        2
      )
    )
  }
  for (const entry of operations) {
    const id = entry.operation.operationId
    if (!titles[id]) throw new Error(`Missing Chinese operation title: ${id}`)
    if (!sdk.includes(`export function ${id}(`))
      throw new Error(`Missing SDK operation: ${id}`)
    codeSamples(document, entry)
    const english = id.replace(/([a-z0-9])([A-Z])/g, "$1 $2").toLowerCase()
    const title =
      lang === "zh" ? titles[id] : english[0].toUpperCase() + english.slice(1)
    const content = `---\ntitle: ${JSON.stringify(title)}\ndescription: ${JSON.stringify(entry.operation.summary ?? "")}\nfull: true\n---\n\n<OperationReference operationId=${JSON.stringify(id)} lang=${JSON.stringify(lang)} />\n`
    await writeFile(`${output}${entry.tag}/${entry.slug}${suffix}.mdx`, content)
  }
}

console.log(
  `Generated ${operations.length} API operations in Chinese and English.`
)
