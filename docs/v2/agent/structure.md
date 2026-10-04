# Agent Crates 与目录提案

下面是新 Agent 的建议布局。**它区分进程、Rust 库、plugin provider 和动态库产物，尚未创建这些目录或修改当前 workspace。** 具体拆分以职责、复用和独立测试为依据，不要求把每个对象或临时阶段拆成 crate。

## 应用

```text
apps/
  meshway-controller/       # 已有的新控制面服务端
  meshwayd/                 # 建议 package: meshway-daemon，binary: meshwayd
    src/
      main.rs               # 启动参数与进程信号
      app/mod.rs            # 宿主装配与退出
      ipc/mod.rs            # 本机身份检查和命令分派
      persistence/mod.rs    # 私有状态布局与恢复协调
      plugins/mod.rs        # 本进程的 provider 装配
  meshway/                  # 建议 package: meshway-cli，binary: meshway
    src/
      main.rs
      commands/mod.rs       # up / down / ping / status、网络选择
      output/mod.rs         # 人类／机器可读输出
```

CLI 不依赖 daemon App 或数据面实现，daemon 不依赖 CLI。两者共享 `meshway-ipc` 的类型／客户端接口；本机 IPC 的具体传输适配可先在该库内部按平台分模块，不按每条命令拆库。

宿主只持有装配需要的状态，领域控制循环由相应 provider 负责。daemon 状态输出和 CLI 输出格式分离，避免把日志字符串当稳定接口。

## 共享库与领域实现

沿用 Cargo 包名 `meshway-*`、目录短名的约定：

```text
crates/
  model/                    # 已有共享 ID、实体、State 值
  configuration/            # 已有配置、执行类型、校验／编译／delta
  transport-spec/           # 已有纯协议字段与验证
  protocol/                 # 已有 Controller HTTP／SSE 合同
  control-client/           # 已有生成 Controller 客户端
  serde/                    # 已有共同严格解码

  ipc/                      # meshway-ipc：本机命令、查询、事件与客户端
  plugin/                   # meshway-plugin：SDK 与共同插件合同
  plugin-host/              # 宿主加载／版本检查及受审 native loader
  control/                  # meshway-control：节点控制面连接 provider
  dataplane/                # Mesh 身份、端到端保护、准入与包流
  connectivity/             # 邻接、候选、主备、打洞生命周期
  underlay/                 # 底层保护与系统资源 owner
  transport/                # 承载领域公共接口
  transport-udp/
  transport-tcp/
  transport-vless/
  transport-trojan/
  tun/
  netstack/
  dns/
  diagnostics/

plugins/                    # 建议：受审 native 导出薄入口／打包适配
```

`plugins/` 不再实现业务或定义第二份 DTO；其职责是把安全 Rust 实现接到约定 ABI 并产出动态库。公共入口代码应由同一 SDK／生成机制复用，具体 Cargo 构建方式待定。native loader 和导出入口的 unsafe 单独审查，领域库及应用保持其安全边界。

各组件可以提供安全 Rust library 供单独测试或将来的静态宿主使用。动态产物不意味着必须通过 ABI 才能做所有单元测试，也不意味着 Apple 当前已能动态加载这些插件。Apple 新增开发仍暂停，其后续宿主采用相同领域合同并按平台决定装载方式。

## 依赖方向

```text
CLI ──► ipc + 必要共享值类型
daemon ──► plugin host + ipc + 私有存储／装配

control ──► control-client + model + configuration
dataplane ──► 执行值类型 + transport／connectivity 的窄接口
connectivity ──► 承载与 underlay 的窄接口
transport provider ──► transport + transport-spec + 所需 host services
tun / netstack ──► 共同包接口 + 本平台实现
dns / diagnostics ──► 共同契约 + 所需 provider 接口
```

跨动态库的交互经过版本化 ABI；图中的 Rust 类型依赖不表示可直接把 Rust 对象地址跨库传递。依赖的是公共合同，不是另一插件的 App 或私有可变容器。实际 provider 依赖由宿主解析，插件不自行找到任意同名动态库加载。

Controller 的 SQL、账户权限与网络策略编译实现不成为节点 provider 的依赖。运行库也不直接依赖 HTTP handler／生成 endpoint：control 将已验证输入交给相应领域接口。

## 模块与测试

- 单文件模块使用 `foo.rs`；存在子模块时使用 `foo/mod.rs`。
- 私有实现测试属于对应模块；公共 API 黑盒测试使用 Cargo 标准的 `tests/*.rs` 或 `tests/<suite>/main.rs`。
- ABI 兼容、buffer／回调归属、取消与正常退出有独立边界测试；实际 provider 仍需协议和资源行为测试。
- CLI／IPC 测试不以启动整个网络代替消息合同验证；组网验收则必须经过正常控制面、daemon 生命周期和真实包流。
- 审查旧回归时保留仍适用的失败反例；已废弃的 profile／旧 DTO fixture 不作为新结构必须保留的理由。

## 现有代码的去向

`apps/agent`、`crates/agent-*` 以及 `automatic-connectivity` 中的实现暂作参考资产。对每个能力判断：职责是否仍成立、接口是否独立于旧合同、资源生命周期能否简化、回归是否仍有效。保留实现须同时保留对应的真实行为保障；重新实现也须重新通过这些反例和端到端验收。

本轮先确定文档和接口方向，尚未授权按这张目录图机械搬迁、创建同名替代 crate 或把旧 NAT／出口分支直接并回 main。
