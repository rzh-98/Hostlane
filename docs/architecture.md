# 架构与协议

## 目录结构

```text
Hostlane/
├─ Cargo.toml                    Rust workspace
├─ .cargo/config.toml            ts-rs 的类型导出目录
├─ crates/
│  ├─ hostlane-protocol/         线上传输的类型：JSON-RPC 帧、错误码、各方法的参数和结果
│  ├─ hostlane-core/             守护进程与客户端
│  │  └─ src/
│  │     ├─ server.rs / client.rs   socket 服务端和客户端（含事件订阅）
│  │     ├─ handlers.rs             方法分发；受管控的方法在这里经过策略检查并写审计日志
│  │     ├─ state.rs                守护进程状态（`Daemon`）
│  │     ├─ catalog.rs              主机目录：合并 ~/.ssh/config、手动添加的主机和本机标签，按名称解析主机
│  │     ├─ ssh_config.rs           解析 ~/.ssh/config
│  │     ├─ hosts.rs                hosts.add/update/remove、secrets.*：校验、保存、删除时清理隧道和密码
│  │     ├─ secrets.rs              密码和口令的存取：系统钥匙串（keyring），测试时可用文件
│  │     ├─ known_hosts.rs          解析和写入 known_hosts、计算指纹
│  │     ├─ ssh.rs                  russh 客户端：指纹校验、认证
│  │     ├─ pool.rs                 连接池：每台主机一条连接、ProxyJump、保活和空闲回收
│  │     ├─ exec.rs                 exec.run / output.read：超时、截断、输出落盘
│  │     ├─ files.rs                files.*：SFTP 读写、搜索、原子替换、备份和撤销
│  │     ├─ patch.rs                文本修改（精确替换、unified diff）和 diff 摘要
│  │     ├─ ports/                  ports.*：端口发现（discover）、HTTP/HTTPS 探测（probe）、证书解析（cert）
│  │     ├─ tunnels.rs              tunnels.*：本地转发、冲突检测、断线重连、临时隧道到期关闭
│  │     ├─ services.rs             services.* / processes.kill：从参数生成 journalctl、systemctl、docker、kill 命令并执行
│  │     ├─ jobs.rs                 jobs.*：后台任务（tmux / nohup）的启动、状态、日志、取消和后台巡检
│  │     ├─ clients.rs              clients.* 和每个请求的令牌校验（A5）
│  │     ├─ remote.rs               Hostlane 自己运行的辅助命令（搜索、mv 兜底、端口扫描）
│  │     ├─ policy/                 策略：shell 词法（shell）、风险分级（classify）、规则匹配（rules）、审批队列（approvals）、急停和放行流程（mod）
│  │     ├─ store.rs                SQLite：审计日志（只追加）、命令记录、文件修改记录、端口备注、保存的隧道、策略规则、主机标签、手动添加的主机、凭据清单、后台任务、登记的客户端和设置
│  │     └─ events.rs               事件总线（broadcast）
│  └─ hostlane-cli/              `hostlane` 二进制
│     └─ src/
│        ├─ main.rs                 CLI 和 `daemon` 子命令
│        ├─ mcp.rs                  `hostlane mcp`：stdio MCP Server（rmcp），工具到方法的映射
│        ├─ manage.rs               `host`、`secret(s)`、`logs`、`service`、`kill`
│        ├─ jobs.rs                 `job start/status/logs/cancel`、`jobs`
│        ├─ clients.rs              `client add/revoke/require-token`、`clients`
│        └─ service.rs              `daemon install`：launchd / systemd --user 服务文件
├─ apps/desktop/                 Electron 桌面端（基于 Desktop-Starter）
│  └─ src/
│     ├─ main/daemon.ts          连接守护进程、按需拉起、保持事件订阅并转发给渲染进程
│     ├─ main/setup.ts           把 CLI 链接到 ~/.local/bin、查看和注册后台服务（调用 hostlane --json daemon …）
│     ├─ main/paths.ts           socket 路径规则（与 hostlane-core/src/paths.rs 保持一致）
│     ├─ shared/ipc.ts           IPC 通道、渲染进程可调用的方法白名单
│     ├─ shared/protocol/        由 ts-rs 生成，不要手改
│     └─ renderer/src/app/       守护进程状态、事件、连接状态等跨页面的 store
├─ scripts/
│  ├─ testbed.py                在 127.0.0.1 上起一个临时 sshd，并把 HOSTLANE_* 指向临时目录
│  ├─ e2e.py                    用 CLI 和 MCP 跑一遍完整流程（CI 中运行）
│  └─ bench.py                  性能实测
├─ .github/workflows/            ci.yml（检查、端到端测试、Windows 编译检查）、release.yml（打 tag 发布）
└─ docs/
```

依赖方向：`hostlane-cli → hostlane-core → hostlane-protocol`。桌面端只依赖由 `hostlane-protocol` 生成的 TS 类型，不直接依赖 Rust 代码。

## 进程模型

```text
agent ──shell──► hostlane（CLI，每次调用一个进程）──┐
agent ──stdio──► hostlane mcp（随 agent 启停）─────┤ 本地 socket
Electron 主进程 ◄─IPC─ 渲染进程（沙箱）─────────────┤ JSON-RPC
                                                    ▼
                                     hostlane daemon run（常驻）
```

- 守护进程是唯一持有 SSH 连接、隧道和审批队列的进程。审批队列只在内存中，守护进程重启后清空。每台主机复用一条 SSH 连接，每条连接最多同时打开 8 个 channel；空闲 10 分钟后断开，每 30 秒 ping 一次以测量延迟和发现断线。
- SSH 凭据只在守护进程中使用，认证顺序：
  1. ssh-agent（`IdentityFile` 对应的密钥优先）。
  2. 私钥文件。加密的私钥用钥匙串中保存的口令解开；没有保存口令时跳过。
  3. 已保存的登录密码：先试 `password`，再试 `keyboard-interactive`（每个提示都回答这个密码）。
- 全部失败时返回 `AUTH_FAILED`，`details`（`AuthFailure`）列出服务器还接受的认证方式、缺少口令的密钥、试过的密钥数量和是否保存了密码，`hint` 给出对应的 `hostlane secret set` 命令。
- 密码和口令保存在系统钥匙串中（macOS 钥匙串、Linux Secret Service、Windows 凭据管理器），服务名为 `Hostlane`，账户名为 `password/<主机>` 或 `passphrase/<密钥的绝对路径>`；同一个密钥的口令由所有使用它的主机共用。SQLite 的 `secrets` 表只记录保存了哪些凭据和保存时间。凭据不会出现在任何方法的结果、审计日志和守护进程日志中：`secrets.set` 的 `secret` 在审计中记为 `<redacted>`，守护进程用完即清零（`zeroize`）。桌面端把输入框中的密码直接通过 IPC 发给守护进程，渲染进程不保存。`HOSTLANE_SECRET_STORE=file` 改用 `<数据目录>/secrets.json`（权限 0600，明文），只供测试使用。
- `hostlane daemon start` 会以独立进程组在后台启动 `hostlane daemon run`，日志写入 `<数据目录>/daemon.log`，等到 socket 能响应后才返回。
- 桌面端启动时，如果守护进程没有运行，会调用 `hostlane daemon start`。关闭桌面端不会停止守护进程。
- `hostlane daemon install` 把守护进程注册为用户级服务：macOS 写入 `~/Library/LaunchAgents/com.hostlane.daemon.plist` 并 `launchctl bootstrap`（`RunAtLoad`，`KeepAlive` 只在非 0 退出时重启）；Linux 写入 `~/.config/systemd/user/hostlane.service` 并 `enable` + `restart`（`Restart=on-failure`）。服务文件记录当前二进制的绝对路径和当时设置的 `HOSTLANE_SOCKET`、`HOSTLANE_DATA_DIR`、`HOSTLANE_SSH_CONFIG`、`HOSTLANE_NOTIFY`。正在运行的守护进程会先被停止，再由服务接管。`--print` 只输出服务文件。
- 服务已安装时，`hostlane daemon start`（以及桌面端和 MCP 的自动拉起）改为让服务管理器启动（`launchctl kickstart` / `systemctl --user start`），不会在服务之外再起一个守护进程。`hostlane daemon stop` 正常退出（退出码 0），服务不会把它重新拉起。
- `hostlane daemon uninstall` 停止服务（守护进程随之退出）并删除服务文件；`hostlane daemon service` 查看是否安装以及服务运行的二进制。
- 打包后的桌面端把 release 版 `hostlane` 放在 `Resources/bin/`（electron-builder 的 `extraResources`，`npm run build:cli` 构建）；`main/daemon.ts` 在打包时使用它，开发时使用 `target/debug/hostlane`，也可以用 `HOSTLANE_BIN` 指定。Windows 版暂不内置（守护进程需要 Unix socket）。

## 路径

| 平台    | 数据目录                                                     | Socket                                                                           |
| ------- | ------------------------------------------------------------ | -------------------------------------------------------------------------------- |
| macOS   | `~/Library/Application Support/Hostlane`                     | `<数据目录>/hostlane.sock`                                                       |
| Linux   | `$XDG_STATE_HOME/hostlane`，默认为 `~/.local/state/hostlane` | `$XDG_RUNTIME_DIR/hostlane/hostlane.sock`，没有时使用 `<数据目录>/hostlane.sock` |
| Windows | `%APPDATA%\Hostlane`                                         | 命名管道（V1）                                                                   |

数据目录中的文件：

| 文件                                         | 内容                                                                                                                                                                                                                                                                                                                                                                                                          |
| -------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `hostlane.db`                                | SQLite（WAL）：`calls` 表是审计日志，由触发器禁止 UPDATE 和 DELETE；`execs` 表是会话监视器使用的命令记录；`file_changes` 表记录每次文件修改及其 diff；`port_notes` 表保存端口备注；`tunnels` 表保存隧道配置；`manual_hosts` 表保存手动添加的主机；`secrets` 表是已保存凭据的清单（不含凭据本身）；`jobs` 表记录后台任务最近一次读到的状态；`clients` 表保存登记的客户端和令牌的 SHA-256。当前 schema 版本为 7 |
| `backups/<change_id>`                        | 文件被修改、上传覆盖或删除前的内容，权限 0600，保留 30 天，供 `files.revert` 使用                                                                                                                                                                                                                                                                                                                             |
| `outputs/<id>.stdout`、`outputs/<id>.stderr` | 命令的完整输出，权限 0600；每个流最多保存 64 MiB，保留 30 天                                                                                                                                                                                                                                                                                                                                                  |
| `known_hosts`                                | Hostlane 信任的主机密钥。Hostlane 也会读取 ssh_config 同目录下的 `known_hosts`（或 `UserKnownHostsFile`），但只写这个文件                                                                                                                                                                                                                                                                                     |
| `daemon.log`                                 | `hostlane daemon start` 启动时的日志                                                                                                                                                                                                                                                                                                                                                                          |
| `secrets.json`                               | 只在 `HOSTLANE_SECRET_STORE=file` 时使用（测试），权限 0600                                                                                                                                                                                                                                                                                                                                                   |

socket 文件权限为 0600，所在目录权限为 0700。启动时如果发现残留的 socket 文件，会先尝试连接：能连上说明已有守护进程在运行，此时拒绝启动；连不上则删除残留文件。

## 协议

换行分隔的 JSON-RPC 2.0：每行一个请求或响应，单行最大 8 MiB。

```json
→ {"jsonrpc":"2.0","id":1,"method":"hosts.list"}
← {"jsonrpc":"2.0","id":1,"result":{"hosts":[…],"warnings":[]}}
← {"jsonrpc":"2.0","id":2,"error":{"code":-32601,"message":"unknown method `x`","data":{"kind":"METHOD_NOT_FOUND","hint":"available methods: …"}}}
```

- 调用方根据 `error.data.kind` 判断错误类型；`hint` 是写给 agent 的下一步建议。
- 错误码：标准 JSON-RPC 错误使用 -32700 到 -32603，Hostlane 自定义错误从 -32000 开始（见 `crates/hostlane-protocol/src/error.rs`）。
- 部分错误在 `error.data.details` 中带有结构化信息，例如 `HOST_KEY_UNKNOWN` 带主机密钥（`HostKeyInfo`），`CONNECTION_LOST` 带 `output_id`。
- 协议发生不兼容的变更时，递增 `PROTOCOL_VERSION`，`daemon.status` 会返回该版本号。

### 调用方标识

请求可以带一个顶层成员 `client`（Hostlane 对 JSON-RPC 的扩展），用于审计和会话监视器区分来源：

```json
{"jsonrpc":"2.0","id":1,"method":"exec.run","params":{…},"client":{"name":"claude-code","session":"a1b2"}}
```

请求还可以带顶层成员 `token`（客户端令牌，见 A5）。守护进程在处理每个请求（包括 `events.subscribe`）之前先校验它：

- 令牌有效时，`client.name` 改为登记的名称，并设置 `client.verified = true`（只由守护进程设置，客户端传入的值会被忽略）；审计日志和命令记录中保存 `verified`。
- 令牌无效或已吊销时返回 `UNAUTHORIZED`。
- 开启 `clients.require_token`（设置项）后，没有令牌的调用一律返回 `UNAUTHORIZED`，例外是 `cli`、`desktop` 调用方和 `daemon.status`。
- `cli`、`desktop`、`unknown` 是保留名，不能登记。令牌格式为 `hlt_` 加 64 位十六进制数，只在创建时返回一次，库中只保存 SHA-256；`last_used_at` 最多每分钟更新一次。
- `clients.create/revoke/configure` 只接受 `desktop` 或 `cli` 调用方，写入审计日志（不含令牌）。

CLI 从环境变量 `HOSTLANE_CLIENT`（默认 `cli`）、`HOSTLANE_SESSION` 和 `HOSTLANE_TOKEN` 读取；桌面端固定为 `desktop`。`hostlane mcp` 使用 `HOSTLANE_CLIENT`（默认 `mcp`，不接受 `cli` 和 `desktop`），会话取 `HOSTLANE_SESSION`，没有时用 MCP 客户端名加进程号（例如 `claude-code-4121`）。

### 事件订阅

`events.subscribe` 必须是连接上的唯一请求。守护进程先返回 `{"result":{"subscribed":true}}`，之后持续推送通知，直到客户端断开：

```json
← {"jsonrpc":"2.0","method":"event","params":{"type":"exec_output","id":"ex-…","stream":"stdout","data":"…"}}
```

事件类型（`Event`）：`exec_started`、`exec_output`、`exec_finished`、`connection`（连接状态变化）、`file_changed`（文件被修改，不含 diff）、`audit_appended`（新增一条审计记录）、`tunnel`（隧道建立或状态变化；流量计数变化时每 2 秒推送一次）、`tunnel_removed`（临时隧道关闭或隧道被删除）、`approval_requested`（新的待审批调用）、`approval_updated`（审批被决定、过期、被使用，或调用方开始/停止等待）、`policy`（急停状态或待审批数量变化）、`job`（后台任务启动或状态变化），以及 `lagged`（订阅方处理太慢，丢了 `skipped` 条事件，应重新拉取状态）。每条命令的每个流最多推送 1 MiB，完整内容用 `output.read` 读取。

### 已实现的方法

| 方法                  | 参数                     | 结果                           | 审计 |
| --------------------- | ------------------------ | ------------------------------ | ---- |
| `daemon.status`       | 无                       | `DaemonStatus`                 |      |
| `daemon.shutdown`     | 无                       | `null`                         |      |
| `hosts.list`          | 无                       | `HostsList`                    |      |
| `hosts.connect`       | `HostParams`             | `Connection`                   | ✓    |
| `hosts.disconnect`    | `HostParams`             | `{"disconnected":bool}`        | ✓    |
| `hosts.trust`         | `TrustParams`            | `TrustResult`                  | ✓    |
| `hosts.label`         | `HostLabelParams`        | `Host`                         | ✓    |
| `hosts.add`           | `HostAddParams`          | `Host`                         | ✓    |
| `hosts.update`        | `HostAddParams`          | `Host`                         | ✓    |
| `hosts.remove`        | `HostParams`             | `HostRemoved`                  | ✓    |
| `secrets.list`        | 无                       | `SecretsList`                  |      |
| `secrets.set`         | `SecretSetParams`        | `SecretChanged`                | ✓    |
| `secrets.delete`      | `SecretDeleteParams`     | `SecretChanged`                | ✓    |
| `connections.list`    | 无                       | `ConnectionsList`              |      |
| `exec.run`            | `ExecParams`             | `ExecResult`                   | ✓    |
| `exec.list`           | `ExecListParams`         | `ExecList`                     |      |
| `output.read`         | `OutputReadParams`       | `OutputChunk`                  |      |
| `files.read`          | `FileReadParams`         | `FileContent`                  | ✓    |
| `files.list`          | `FileListParams`         | `FileList`                     | ✓    |
| `files.search`        | `FileSearchParams`       | `FileSearchResult`             | ✓    |
| `files.write`         | `FileWriteParams`        | `FileChangeResult`             | ✓    |
| `files.patch`         | `FilePatchParams`        | `FileChangeResult`             | ✓    |
| `files.upload`        | `FileUploadParams`       | `FileChangeResult`             | ✓    |
| `files.download`      | `FileDownloadParams`     | `FileDownloadResult`           | ✓    |
| `files.change`        | `FileChangeParams`       | `FileChange`（含 diff）        |      |
| `files.revert`        | `FileChangeParams`       | `FileChangeResult`             | ✓    |
| `ports.list`          | `PortsListParams`        | `PortsList`                    | ✓    |
| `ports.inspect`       | `PortParams`             | `PortsList`（只含该端口）      | ✓    |
| `ports.check`         | `PortParams`             | `PortCheck`                    | ✓    |
| `ports.find_free`     | `FindFreeParams`         | `FreePorts`                    | ✓    |
| `ports.annotate`      | `PortAnnotateParams`     | `PortAnnotated`                | ✓    |
| `tunnels.list`        | 无                       | `TunnelsList`                  |      |
| `tunnels.open`        | `TunnelOpenParams`       | `TunnelOpened`                 | ✓    |
| `tunnels.close`       | `TunnelCloseParams`      | `TunnelsClosed`                | ✓    |
| `tunnels.start`       | `TunnelParams`           | `TunnelOpened`                 | ✓    |
| `tunnels.update`      | `TunnelUpdateParams`     | `TunnelOpened`                 | ✓    |
| `tunnels.delete`      | `TunnelParams`           | `TunnelsClosed`                | ✓    |
| `services.logs`       | `ServiceLogsParams`      | `ServiceCommandResult`         | ✓    |
| `services.start`      | `ServiceControlParams`   | `ServiceCommandResult`         | ✓    |
| `services.stop`       | `ServiceControlParams`   | `ServiceCommandResult`         | ✓    |
| `services.restart`    | `ServiceControlParams`   | `ServiceCommandResult`         | ✓    |
| `processes.kill`      | `ProcessKillParams`      | `ServiceCommandResult`         | ✓    |
| `audit.list`          | `AuditListParams`        | `AuditList`                    |      |
| `approvals.list`      | `ApprovalsListParams`    | `ApprovalsList`                |      |
| `approvals.get`       | `IdParams`               | `Approval`                     |      |
| `approvals.resolve`   | `ApprovalResolveParams`  | `Approval`                     | ✓    |
| `policy.status`       | 无                       | `PolicyStatus`                 |      |
| `policy.stop`         | `PolicyStopParams`       | `PolicyStopped`                | ✓    |
| `policy.resume`       | 无                       | `PolicyStatus`                 | ✓    |
| `policy.check`        | `PolicyCheckParams`      | `PolicyCheck`                  |      |
| `policy.rules.list`   | 无                       | `PolicyRules`                  |      |
| `policy.rules.add`    | `PolicyRuleAddParams`    | `PolicyRule`                   | ✓    |
| `policy.rules.delete` | `IdParams`               | `{"deleted":id}`               | ✓    |
| `jobs.start`          | `JobStartParams`         | `Job`                          | ✓    |
| `jobs.list`           | `JobsListParams`         | `JobsList`                     |      |
| `jobs.status`         | `JobStatusParams`        | `Job`                          |      |
| `jobs.logs`           | `JobLogsParams`          | `JobLogs`                      | ✓    |
| `jobs.cancel`         | `JobParams`              | `Job`                          | ✓    |
| `clients.list`        | 无                       | `ClientsList`                  |      |
| `clients.create`      | `ClientNameParams`       | `ClientCreated`                | ✓    |
| `clients.revoke`      | `ClientNameParams`       | `RegisteredClient`             | ✓    |
| `clients.configure`   | `ClientsConfigureParams` | `ClientsList`                  | ✓    |
| `events.subscribe`    | 无                       | `Subscribed`，之后推送 `Event` |      |

审计列打勾的方法中，访问服务器的那些（`method::GATED`：`hosts.connect` 到 `processes.kill`，以及 `jobs.start/logs/cancel`；不含主机、凭据和客户端管理）还要先经过策略检查，见[策略与审批](#策略与审批)。

`exec.run` 的要点：

- `cwd` 和 `env` 会被拼成 `cd '…' || exit 1; export NAME='…'; <command>`，命令由远程用户的登录 shell 执行。
- 超时（默认 120 秒，最长 6 小时）后先发 TERM，2 秒后关闭 channel，状态为 `timed_out`。
- 默认每个流最多返回 16 KiB（`max_output_bytes`，256 B 到 1 MiB），超出时保留头尾并在中间注明省略了多少字节，同时 `truncated` 为 true；完整输出用 `output.read` 按偏移读取，负偏移表示从末尾倒数。
- 主机指纹未知或变化时直接失败（`HOST_KEY_UNKNOWN` / `HOST_KEY_CHANGED`），不会自动信任。`hosts.trust` 要求传入与服务器当前密钥一致的指纹；指纹变化时 `hosts.trust` 同样拒绝，必须由人删除旧记录。
- 审计日志记录参数和结果摘要（不含 stdout/stderr），`stdin` 只记录字节数。守护进程重启时，上次未结束的命令会被标为 `interrupted`；急停时，agent 正在运行的命令会收到 TERM，状态为 `cancelled`。

文件操作（`files.*`）的要点：

- 读写走 SFTP，搜索在服务器上执行 `rg`（没有时用 `grep -rnI`）。远程路径可以是绝对路径，也可以相对于登录用户的 home 目录（`~/…` 同样可用）；`upload` / `download` 的本地路径必须是绝对路径，CLI 会按当前目录补全。
- `files.read` 返回整个文件的 `sha256`（与读取的行范围无关）。默认每次最多 2000 行、256 KiB，`truncated` 为 true 时用 `offset = end_line + 1` 继续读；超过 16 MiB 的文件请用 `download`。非 UTF-8 文件只返回哈希，`binary` 为 true。
- 写类操作（`write`、`patch`、`upload`、`revert`）的流程：
  1. 解析目标路径；符号链接会被跟随，写入链接指向的文件，链接本身保持不变。
  2. 读取当前内容，同时计算 `sha256`，并复制到 `backups/<change_id>`。
  3. 如果传了 `expected_sha256` 且与当前哈希不一致，返回 `FILE_CHANGED`，`details` 中带 `current_sha256`。替换已有文件时必须传 `expected_sha256` 或 `overwrite: true`（`patch` 除外，它本身要按内容匹配）。
  4. 在同一目录写临时文件 `.<name>.hostlane-<id>`（初始权限 0600），复制原文件的属主（失败时在 `warnings` 中说明）和权限；新文件默认 0644，可用 `mode` 指定。
  5. 替换前再检查一次大小和修改时间，然后用 `posix-rename@openssh.com` 原子替换；服务器不支持该扩展时退回执行 `mv -f`。任何一步失败都会删除临时文件，原文件不受影响。
  6. 记录到 `file_changes` 表（含 diff，最多 1 MiB），推送 `file_changed` 事件，返回 `change_id`、新哈希和增删行数。
- `files.patch` 接受 `edits`（`old_text` 必须恰好出现一次，除非 `replace_all`）或单个文件的 unified `diff`。diff 的 hunk 按内容定位，行号可以不准；不匹配时返回 `PATCH_FAILED`，`details` 指出是哪一处。文件使用 CRLF 时，编辑和 diff 会自动适配。
- `files.revert` 把文件恢复到某次修改之前：该修改新建了文件时删除它，否则写回备份。文件在那之后又被改过时返回 `FILE_CHANGED`，需要先撤销更晚的修改。撤销本身也是一次修改，可以再撤销。
- 所有操作都以 SSH 用户的身份执行，不使用 sudo；没有权限时返回 `PERMISSION_DENIED`。
- 审计日志不记录文件内容：`content` 替换为字节数和 `sha256`，`edits` / `diff` 只记录数量或大小，读取结果去掉 `content`，列表和搜索结果只记录条数。diff 用 `files.change` 查看。

端口（`ports.*`）的要点：

- 一次扫描只在服务器上运行两条命令，不安装任何东西：
  - 第一条列出监听的 TCP socket 和容器。Linux 上依次尝试 `ss -tlnp`、`netstat -tlnp`、`/proc/net/tcp*`；macOS/BSD 用 `netstat -an` 加 `lsof`；装了 docker 时再运行 `docker ps`。
  - 第二条对持有 socket 的进程运行 `ps`（PID、父进程、用户、运行时长、命令行），并读取 `/proc/<pid>/cgroup`。
- 同一端口的多个 socket（如 `0.0.0.0` 和 `::`）合并为一项。主进程是父进程不在同一端口上的那个（nginx master 而不是 worker）。
- 服务关联的顺序：
  1. cgroup 指向容器时，关联到该容器。
  2. 端口是某个容器发布的端口时（包括 docker-proxy 和只有 iptables 规则、没有 socket 的情况），关联到该容器，并给出容器内端口和 compose 项目/服务。
  3. cgroup 是 `*.service` 时，关联到 systemd 单元，并区分用户单元。
- `bind` 由绑定地址推断可达范围（`loopback` / `private` / `public` / `all`），不考虑防火墙。
- SSH 用户不是 root 时，其他用户的进程看不到，`warnings` 中会说明。
- 探测通过 SSH 的 `direct-tcpip` 通道从服务器本机连接端口，端口不需要对外开放；需要 sshd 允许 TCP 转发（`AllowTcpForwarding`，默认开启），不允许时 `warnings` 中会说明。
  - 发送 `GET /`：先发明文请求，443、8443 等端口先试 TLS。
  - 服务器先发问候语的协议（SSH、SMTP、FTP、MySQL、Redis 等）只报告协议名。
  - nginx/Go 对 HTTPS 端口的明文请求返回的 400 会触发 TLS 重试。连接后一直不响应的端口不再尝试 TLS。
  - TLS 接受任何证书，只读取主体、签发者、SAN、有效期和是否自签名。
  - 每个端口每次尝试 3 秒，最多 8 个并发，最多探测 64 个端口。
  - `ports.inspect` 可以传 `server_name`，作为 Host 头和 SNI。
- 备注（`ports.annotate`）保存在本机的 `port_notes` 表中，不会写到服务器上。有备注但没有在监听的端口也会出现在列表中（`listening: false`）。`ports.find_free` 和 `ports.check` 的建议端口会跳过正在监听、Docker 已发布和有备注的端口。
- 审计日志中，端口列表只记录条数。

主机和凭据（`hosts.add/update/remove`、`secrets.*`）的要点：

- 手动添加的主机保存在 `manual_hosts` 表中，`source` 为 `manual`。`~/.ssh/config` 中的 `Host *` 等通配块同样作用于它们；与 `Host` 块重名时以 `~/.ssh/config` 为准，`hosts.list` 的 `warnings` 中会说明。`hosts.list` 的 `manual` 字段返回手动主机保存时的原始设置，供编辑使用。
- 校验：名称只能包含字母、数字、`.`、`_`、`-`；`hostname` 不能以 `-` 开头、不能含空白；端口不能为 0；指定的密钥文件必须存在。指定 `identity_file` 时只使用这个密钥（等同于 `IdentitiesOnly yes`）。
- `hosts.update` 是整体替换，并断开这台主机现有的连接。`hosts.remove` 同时删除这台主机的保存隧道和登录密码，`hosts.update` / `remove` 不接受来自 `~/.ssh/config` 的主机。
- `secrets.set` 保存口令前会先用它解开密钥，口令错误或密钥没有加密时返回 `INVALID_PARAMS`。不传 `key_path` 时，使用这台主机第一个加密的密钥。
- `hosts.add/update/remove` 和 `secrets.set/delete` 只接受 `desktop` 或 `cli` 调用方，不经过策略检查，但会写入审计日志。

服务和进程（`services.*`、`processes.kill`）的要点：

- 守护进程根据参数生成命令，不接受任意 shell：
  - `services.logs`：systemd 单元用 `journalctl [--user] -u <name> -n <lines> --no-pager -o short-iso [--since -10min]`，容器用 `docker logs --tail <lines> --timestamps [--since 10m] <name> 2>&1`。默认 200 行，最多 10000 行。
  - `services.start/stop/restart`：`systemctl [--user] <action> <name>` 或 `docker <action> <name>`。SSH 用户不是 root 时，系统单元默认加 `sudo -n`（不会输入密码），可以用 `sudo: false` 关闭。
  - `processes.kill`：`kill -<SIGNAL> <pid>`，pid 必须大于 1，信号只能是 TERM、INT、HUP、KILL、QUIT、USR1、USR2。
- 名称只允许 `[A-Za-z0-9@._:-]` 且不以 `-` 开头，`since` 只接受 `30s`、`10m`、`2h`、`1d` 这样的值。参数不合法时，在策略检查之前就返回 `INVALID_PARAMS`。
- 策略按生成的命令分级（与 `exec.run` 相同），审批界面显示的也是这条命令。命令通过 `exec.run` 的同一条路径执行，会出现在会话监视器中，结果包含 `command`、`ExecResult` 的全部字段，以及识别出的失败原因 `hint`（sudo 需要密码、无权访问 Docker 或 journal 等）。

隧道（`tunnels.*`）的要点：

- 只支持本地转发（`ssh -L`）：守护进程在本机监听，每个进来的连接通过主机连接池中的 SSH 连接打开一个 `direct-tcpip` 通道，转发到 `remote_host:remote_port`（默认 `127.0.0.1`，即服务器本机）。转发通道不占用每条连接 8 个 channel 的配额。运行中的隧道会一直持有这条 SSH 连接，所以连接池不会把它当作空闲连接关闭。
- 本机端口默认只绑定 `127.0.0.1`。绑定其他地址（如 `0.0.0.0`）必须传 `allow_public: true`（CLI 为 `--allow-public`），否则返回 `INVALID_PARAMS`。
- 不传 `local_port` 时，优先使用与远程相同的端口（≥ 1024 且空闲时），否则依次尝试其后 20 个端口，最后由系统分配。指定的端口被本机程序或其他隧道（包括已停止的保存隧道）占用时返回 `PORT_IN_USE`，`details` 带 `port` 和 `suggested_port`。判断占用时，除了尝试绑定，还会尝试连接该端口，以发现监听在通配地址上的程序。
- `tunnels.open` 先建立 SSH 连接并试连一次目标：服务器禁止 TCP 转发时返回 `FORWARDING_DENIED`；目标没有在监听只作为 `warnings` 返回，隧道照常建立。
- 状态：`active`（运行中）、`reconnecting`（SSH 连接断开，本机端口保持监听，按 1、2、4…60 秒退避重连；有本机连接进来时立即重试，连接最多等 15 秒）、`failed`（本机端口被占用、主机密钥未信任或已变化、主机不存在，不再重试，修复后用 `tunnels.start`）、`stopped`（已停止的保存隧道）。`hosts.disconnect` 会断开连接，运行中的隧道随后会重新连上。
- 单个转发连接失败（例如目标拒绝连接）不影响隧道状态，原因记在 `last_connection_error`，下一次成功转发时清除。
- 保存的隧道（`save: true`）写入 `tunnels` 表，`auto_start` 默认为 true，守护进程启动时恢复。未保存的隧道只在守护进程运行期间存在；除桌面端外，调用方创建的未保存隧道默认 1 小时后关闭（`ttl_secs`，最长 24 小时），到期关闭会以 `hostlane` 为调用方写入审计日志。`tunnels.close` 传 `session` 时关闭该会话创建的全部临时隧道，agent 应在会话结束时调用。
- `tunnels.close` 停止隧道：临时隧道被移除，保存的隧道变为 `stopped`；`tunnels.delete` 同时删除保存的配置。`tunnels.update` 修改本机端口或绑定地址时，运行中的隧道会换到新端口（失败时恢复原端口）；修改远程地址对之后的新连接生效；`save: true` 会保存临时隧道并取消到期时间。

后台任务（`jobs.*`）的要点：

- `jobs.start` 在服务器上创建 `~/.hostlane/jobs/<id>/`，写入 `cmd`（命令，`cwd` 和 `env` 的拼法与 `exec.run` 相同）和包装脚本 `run.sh`，然后启动包装脚本：有 `tmux` 时放进分离的会话 `hl-<id>`，否则在后台运行并忽略 SIGHUP（相当于 `nohup`）。`backend` 可以强制指定，指定 `tmux` 而服务器没有安装时启动失败。所有辅助命令都用 `sh -c` 执行，不依赖用户的登录 shell。
- 包装脚本把命令的 stdout 和 stderr 合并写入 `out`，把 pid 和进程组写入 `pid`、`pgid`，结束时把退出码写入 `exit`。任务不受 SSH 断线和守护进程重启影响，状态总是从这些文件读回。
- 状态：`running`、`exited`（有 `exit` 文件）、`cancelled`（由 `jobs.cancel` 停止）、`lost`（进程已不在但没有退出码，例如服务器重启）。`jobs.list` 只返回本机记录的最近状态；`jobs.status` 会到服务器上读取，`wait_ms`（最长 120 秒）可以等待任务结束。守护进程每 10 秒检查一次已连接主机上运行中的任务，状态变化时推送 `job` 事件。
- `jobs.logs` 按字节偏移读取 `out`：传 `offset` 从该位置读，否则返回最后 `tail_lines` 行（默认 100）；每次最多 `limit` 字节（默认 64 KiB，最大 1 MiB），按 UTF-8 边界截断。结果中的 `next_offset` 用于下次续读，`complete` 表示任务已结束且已读到末尾；`wait_ms` 可以等待新输出。
- `jobs.cancel` 先向进程组发 TERM，5 秒后仍在运行则发 KILL，再关闭 tmux 会话。`policy.stop` 会取消 agent 启动的全部运行中任务（`PolicyStopped.cancelled_jobs`）。
- 策略：`jobs.start` 按命令分级（与 `exec.run` 相同）；`jobs.logs` 为 L0，`jobs.cancel` 为 L1，二者都按任务所在的主机匹配规则。审计日志记录命令，不记录日志内容。

### Web 运维

审计与回滚（M11）：可选的 `client.task` 来自 CLI/MCP 的 `HOSTLANE_TASK`，不替代会话 ID。schema v8 为调用、命令、文件修改保存任务标识；旧数据无需回填。`audit.list` 支持组合 `task`、`session`、`client`、`host`，CLI 分页导出或按开始时间排序。`files.rollback_plan` 返回指定任务尚未撤销的修改（最新在先），`rollback --apply` 逐条调用原有的 `files.revert`，不绕过审批、哈希校验或备份保留期。整个批次不是跨主机事务，失败后可以重新预览。回放只读取本地保留的输出，不在远端重新执行。

- `hosts.profile`、`web.report` 只对已配置主机运行固定的只读命令，沿用 SSH 指纹校验、策略和审计。报告不主动从公网扫描，不申请 sudo，不返回代理配置原文。不可判断的防火墙状态显示为未知。解析范围为 `/etc/nginx/nginx.conf`、`conf.d/*.conf`、`sites-enabled/*` 和 `/etc/caddy/Caddyfile` 中的静态站点及上游。
- `hosts.note {host, content}` 只允许用户保存备注（最多 16 KiB）；`hosts.export {host}` 返回 `content` 和 `markdown`。备注、健康检查配置复用 SQLite 的 `settings`，不改变数据库版本。
- `health.set {host, port, interval_secs}` 由用户配置，间隔为 10–86400 秒，0 表示移除。`health.list` 返回最近一次状态。守护进程使用连接池探测服务器 `127.0.0.1` 上的 HTTP(S) `/`；2xx/3xx 为正常，连接、超时和响应异常会触发系统通知。同一故障不反复提醒，`HOSTLANE_NOTIFY=0` 禁用通知。
- `reverse.open {host, remote_port, local_port}` 仅供人操作，风险 L2。使用 russh `tcpip-forward` 向 sshd 请求回环监听；远端口可填 0。入站通道必须属于对应 SSH 连接已登记的端口并报告回环来源，只连接指定的本机回环端口。`reverse.list` 列出临时映射；`reverse.close {host, port}` 同时撤销本地访问和远端监听。它不同于 `tunnels.open` 的本地转发，不持久化、不重连。

### 不可信数据（S6）

含服务器文本的结果是不可信的：服务器上的日志和文件可能包含针对 agent 的提示注入。`hostlane_protocol::untrusted_fields(method)` 列出每个方法中这样的字段：`exec.run`、`services.*`、`processes.kill` 的 `stdout` / `stderr`，`output.read` 和 `jobs.logs` 的 `data`，`files.read` 的 `content`，`files.search` 的 `matches`。守护进程在这些方法的结果中加一个 `untrusted` 成员（字段名数组）；CLI 的 `--json` 输出也带它。MCP 工具结果在 JSON 前面加一段警告（`UNTRUSTED_NOTE`），MCP 的 `instructions` 中也有同样的说明。

### 策略与审批

每次调用受管控的方法（`method::GATED`）时，守护进程按以下顺序处理：

1. **急停**：急停开启时，除桌面端外的调用一律返回 `POLICY_DENIED`（`details.stopped` 为 true）。
2. **ticket**：参数中带 `ticket_id` 且与原调用一致时，使用该审批的结果（见下文）。
3. **风险分级**（`policy/classify.rs`）：
   - `exec.run` 的命令先做 POSIX shell 词法分析，拆出管道、`;`、`&&`、子 shell、命令替换、重定向和 heredoc；`sudo`、`env`、`nohup`、`sh -c`、`bash -s`（分析 stdin）、`xargs`、`find -exec`、`docker exec`、`ssh` 等包装会递归展开。
   - 再按命令判断：读文件、列目录、`git status` 等为 L0；写用户文件、`git commit`、`docker exec` 为 L1；`sudo`、`systemctl restart`、包管理器、写系统路径、`docker rm`、`git reset --hard` 为 L2；`rm -rf` 大范围或系统目录、`mkfs`、`dd` 写块设备、`docker volume rm`、`DROP TABLE` 等为 L3。不认识的命令至少为 L1，无法解析的命令行为 L2。
   - 文件操作按路径分级：写系统路径或含凭据的文件为 L2，其余写入为 L1；读取凭据文件为 L1。
   - 一次调用的风险取所有部分的最高级，`reasons` 列出依据。
4. **决定**（`policy/rules.rs`）：用户规则可以按主机、环境、标签、调用方、方法、命令、路径（glob，`*` 可匹配 `/`）和风险范围匹配。多条规则匹配时，`deny` 优先于 `allow`，`allow` 优先于“本会话授权”，再优先于 `approve`。没有规则匹配时使用默认值：L0 允许；L1 允许（prod 主机需要审批）；L2 需要审批；L3 需要审批（prod 主机拒绝）。桌面端自己的操作总是允许。
5. **审批**：需要审批时创建 `Approval`（文件写入会附带 diff 预览），推送 `approval_requested`。桌面端没有订阅事件时，守护进程直接发系统通知（macOS 用 `osascript`，Linux 用 `notify-send`；设置 `HOSTLANE_NOTIFY=0` 可关闭）；桌面端在运行时由它发通知，点击通知会打开审批页。
   - 默认阻塞等待：调用方的请求保持打开，最多等 `approval_timeout_secs`（默认 120 秒，最长 3600 秒）。超时返回 `APPROVAL_REQUIRED`，审批仍然有效。
   - 参数中带 `async: true` 时立即返回 `APPROVAL_REQUIRED`，`details.ticket_id` 是审批 ID。之后用完全相同的参数加上 `ticket_id` 重试：仍待审批时等待；已允许时执行（每个 ticket 只能用一次）；已拒绝时返回 `POLICY_DENIED`，`hint` 中带拒绝原因。
   - 待审批的 ticket 1 小时后过期，允许后要在 15 分钟内使用；历史保留 1 天、最多 200 条。
   - 用户可以选择本次允许（`once`）、本会话允许（`session`，同一会话在同一主机上的同一命令或路径不再询问，守护进程重启后失效）或总是允许（`always`，生成一条 `allow` 规则，`created_by` 为 `approval <id>`）。

`ticket_id`、`async` 和 `approval_timeout_secs` 由守护进程从参数中取出，不会传给方法本身。CLI 对应全局参数 `--ticket`、`--async` 和 `--approval-timeout`。在交互终端中运行、且调用方为 `cli` 的 CLI 会先以 async 方式调用；需要审批时直接在终端询问，回答后自行审批或拒绝，再用 ticket 重试。直接回车则等待桌面端或 `hostlane approve` 的决定。

其他约定：

- 放行的调用在审计日志中记录 `risk` 和 `approver`（`desktop`、`cli`、`rule <id>` 或 `session grant`），被放行的命令在 `ExecRecord.approver` 中也有记录。
- `approvals.resolve`、`policy.resume`、`policy.rules.add` / `delete`、`hosts.label`、`hosts.add/update/remove`、`secrets.set/delete` 和 `clients.create/revoke/configure` 只接受 `desktop` 或 `cli` 调用方。没有令牌时调用方名称由客户端自己声明，这只能防止 agent 误操作，不是安全边界；登记的 agent 带令牌调用时名称经过验证，开启 `clients.require_token` 后，agent 不带令牌就无法调用。`policy.stop` 任何调用方都可以使用。
- `policy.stop` 拒绝所有待审批调用，关闭 agent 创建的隧道（桌面端创建的、已保存但未运行的隧道不受影响），终止 agent 正在运行的命令，取消 agent 启动的后台任务。急停状态保存在 `settings` 表中，守护进程重启后仍然有效，此时也不会恢复 agent 创建的保存隧道。
- 主机的环境和标签来自 `~/.ssh/config` 中 `Host` 块内的注释 `# hostlane: env=prod tags=web,eu`，也可以用 `hosts.label` 在本机覆盖（保存在 `host_labels` 表中）。
- `policy.check` 返回某个调用的风险和决定，不执行也不创建审批。

### 错误类型

| kind                                                                                   | code            | 含义                                                                  |
| -------------------------------------------------------------------------------------- | --------------- | --------------------------------------------------------------------- |
| `PARSE_ERROR` / `INVALID_REQUEST` / `METHOD_NOT_FOUND` / `INVALID_PARAMS` / `INTERNAL` | -32700 … -32603 | 标准 JSON-RPC 错误                                                    |
| `NOT_IMPLEMENTED`                                                                      | -32000          | 方法已规划但尚未实现                                                  |
| `HOST_NOT_FOUND`                                                                       | -32001          | 主机不在 ssh_config 中；hint 列出已知主机                             |
| `HOST_UNREACHABLE`                                                                     | -32002          | TCP 连接或 SSH 握手失败、超时                                         |
| `APPROVAL_REQUIRED`                                                                    | -32003          | 调用需要审批；`details` 带 `ticket_id`、`risk`、`reasons`             |
| `POLICY_DENIED`                                                                        | -32004          | 被规则、默认策略、审批人或急停拒绝                                    |
| `FILE_CHANGED`                                                                         | -32005          | 文件的当前哈希与预期不符，或文件已存在；`details` 带 `current_sha256` |
| `HOST_KEY_UNKNOWN`                                                                     | -32006          | 主机密钥尚未信任                                                      |
| `HOST_KEY_CHANGED`                                                                     | -32007          | 主机密钥与记录不一致，或已被吊销                                      |
| `AUTH_FAILED`                                                                          | -32008          | 所有认证方式都失败                                                    |
| `CONNECTION_LOST`                                                                      | -32009          | 命令执行中连接断开；部分输出可用 `output.read` 读取                   |
| `OUTPUT_NOT_FOUND`                                                                     | -32010          | `output_id` 不存在或已过期                                            |
| `FILE_NOT_FOUND`                                                                       | -32011          | 远程文件或目录不存在                                                  |
| `PERMISSION_DENIED`                                                                    | -32012          | SSH 用户无权读写该路径                                                |
| `PATCH_FAILED`                                                                         | -32013          | 某处编辑或 hunk 与文件不匹配，没有写入任何内容                        |
| `FILE_ERROR`                                                                           | -32014          | 其他 SFTP 错误，`message` 中带服务器给出的原因                        |
| `REMOTE_COMMAND_FAILED`                                                                | -32015          | Hostlane 在服务器上运行的辅助命令（如端口扫描）失败或超时             |
| `PORT_IN_USE`                                                                          | -32016          | 隧道的本机端口已被占用；`details` 带 `port` 和 `suggested_port`       |
| `TUNNEL_NOT_FOUND`                                                                     | -32017          | 隧道 ID 不存在                                                        |
| `FORWARDING_DENIED`                                                                    | -32018          | SSH 服务器禁止 TCP 转发（`AllowTcpForwarding no`）                    |
| `APPROVAL_NOT_FOUND`                                                                   | -32019          | 审批 ID 不存在（守护进程重启后会清空）                                |
| `RULE_NOT_FOUND`                                                                       | -32020          | 规则 ID 不存在                                                        |
| `UNAUTHORIZED`                                                                         | -32021          | 令牌无效或已吊销，或开启了 `clients.require_token` 而调用没有带令牌   |
| `JOB_NOT_FOUND`                                                                        | -32022          | 后台任务 ID 不存在                                                    |

### 新增一个方法

1. 在 `hostlane-protocol` 中定义参数和结果类型（加上 `#[derive(TS)] #[ts(export)]`），并把方法名加入 `method` 模块。
2. 在 `hostlane-core/src/handlers.rs` 的 `run` 中实现。访问服务器的方法要加入 `method::GATED`，并在 `policy/mod.rs` 的 `subject` 中说明如何分级。
3. 在 CLI 中加对应的子命令（支持 `--json`）；agent 需要用到时，在 `hostlane-cli/src/mcp.rs` 的 `specs` 中加一个工具（只能由人执行的操作不要加）。
4. 如果渲染进程需要调用，把方法名加入 `apps/desktop/src/shared/ipc.ts` 的 `RENDERER_METHODS`。
5. 运行 `cargo test --workspace` 重新生成 TS 类型。

## CLI 约定

- 所有命令都支持 `--json`。成功时 stdout 输出结果；失败时退出码为 1，stdout 输出 `{"error":{"kind","message","hint"}}`。
- 不加 `--json` 时，输出给人看的文本，错误写到 stderr。
- `DAEMON_NOT_RUNNING` 是 CLI 自己产生的错误，表示无法连接 socket。
- `hostlane secret set` 从终端（不回显）或 `--stdin` 读取凭据，不接受命令行参数，以免留在 shell 历史和进程列表中。
- `hostlane exec`、`logs`、`service`、`kill` 不加 `--json` 时，退出码等于远程命令的退出码，超时为 124，被急停终止为 130；加 `--json` 时只要命令开始执行就返回 0，结果看 `status` 和 `exit_code`。

## MCP

`hostlane mcp` 用 [rmcp](https://crates.io/crates/rmcp) 实现 stdio MCP Server，由 agent 启动，生命周期跟随 agent。它不持有任何状态，每次工具调用都通过 `hostlane_core::client::Client` 转发到守护进程，因此策略、审批、审计、急停与 CLI 完全一致。

- **工具映射**：`mcp.rs` 中的 `specs` 是一张表，每个工具对应一个守护进程方法，并声明 JSON Schema。调用时只转发 schema 中声明的参数；受管控的方法（`method::GATED`）额外接受 `async`、`ticket_id` 和 `approval_timeout_secs`。`edit_file` 和 `apply_diff` 都映射到 `files.patch`，分别只暴露 `edits` 和 `diff`。`check_policy` 接受工具名和参数，先翻译成方法和参数再调用 `policy.check`。`hosts` 会顺带返回 `connections.list` 的结果。后台任务对应 `job_start`、`job_status`、`job_logs`、`job_cancel` 和 `list_jobs`。
- **不开放的方法**：`approvals.resolve`、`policy.resume`、`policy.rules.*`、`hosts.trust`、`hosts.label`、`hosts.add/update/remove`、`secrets.*`、`clients.*`、`policy.stop`、`daemon.shutdown`，以及管理保存隧道的 `tunnels.start/update/delete`（agent 只开临时隧道，用 `close_tunnel` 关闭）。单元测试会检查只能由人执行的方法没有被暴露。
- **结果**：成功时返回 `structuredContent`（守护进程的原始结果），`content` 是同一份 JSON 文本；结果带 `untrusted` 时，`content` 前面多一段不可信数据的警告。失败时返回 `isError: true` 的工具结果，内容为 `{"error":{"kind","message","hint","details"}}`，和 CLI 的 `--json` 错误格式相同；未知工具名返回协议错误 `-32602`。
- **审批**：默认阻塞等待（与 CLI 非交互模式相同）；`async: true` 时立即返回 `APPROVAL_REQUIRED` 和 `details.ticket_id`。MCP 客户端可能有自己的工具超时（例如 Codex 的 `tool_timeout_sec`），接入页的配置片段把它设为 600 秒。
- **令牌**：`HOSTLANE_TOKEN` 有值时随每个请求发送，调用方按令牌登记的名称记录。
- **守护进程**：启动时和遇到 `DAEMON_NOT_RUNNING` 时会调用与 `hostlane daemon start` 相同的逻辑拉起守护进程（服务已安装时交给服务管理器），然后重试一次。stdout 只用于 MCP 协议，不输出日志。

## 测试与 CI

- `cargo test --workspace`：单元测试，同时重新生成 `apps/desktop/src/shared/protocol/`。
- `python3 scripts/e2e.py [hostlane]`：在 127.0.0.1 上以当前用户起一个临时 sshd（端口 `$E2E_PORT`，默认 2222；自己的主机密钥、用户密钥和 `sshd_config`），`HOSTLANE_SOCKET`、`HOSTLANE_DATA_DIR`、`HOSTLANE_SSH_CONFIG` 都指向临时目录，然后用 CLI 和 MCP 跑完整流程：指纹校验、exec、输出截断、文件读写和撤销、端口发现和 HTTP 探测、隧道、审批和 ticket、急停、审计、后台任务（nohup；服务器有 `tmux` 时再测 tmux）、客户端令牌、MCP 的不可信数据标记。`E2E_KEEP=1` 保留临时目录。
- `apps/desktop/scripts/smoke.mjs`：启动打包后的桌面端（独立的数据目录和 `--user-data-dir`），检查页面挂载、主机页显示 `ssh_config` 中的主机、内置的守护进程已启动，并截图。
- `.github/workflows/ci.yml`（push 到 main 和 PR）：
  - Linux 和 macOS：`cargo fmt --check`、`cargo clippy -D warnings`、`cargo test`、检查生成的 TS 类型已提交、`e2e.py`；Linux 上再跑 `bench.py`，结果写到 job summary。
  - 桌面端：`npm ci`、`typecheck`、`prettier --check`、`electron-vite build`。
  - Windows：只跑 `cargo check`，允许失败，用来跟踪 M12 的进度。
- `.github/workflows/release.yml`（推送 `v*` tag）：在 macOS（arm64）和 Linux（x64）上打安装包（dmg、zip、AppImage）并跑冒烟测试，另外打包 CLI（`hostlane-<版本>-<target>.tar.gz` 和 `.sha256`），发布到 GitHub Release；tag 带 `-` 时标为预发布。安装包没有签名，macOS 上需要 `xattr -cr Hostlane.app`。

## 性能实测

`python3 scripts/bench.py [hostlane]`（默认 `target/release/hostlane`）使用同一个临时 sshd，服务器就是本机，所以结果反映的是 Hostlane 自身的开销，不含网络延迟。2026-11-25 在 Apple M 系列（10 核）、macOS 上的结果：

| 指标                                              | 结果                        | 目标（需求 §6） |
| ------------------------------------------------- | --------------------------- | --------------- |
| 守护进程空闲内存（RSS）                           | 8.9 MB                      | < 30 MB         |
| `exec.run true`（socket，连接已建立）             | 中位数 4.1 ms，p95 5.6 ms   |                 |
| 其中 SSH channel 上的耗时（`duration_ms`）        | 中位数 4 ms                 |                 |
| OpenSSH `ssh test true`，复用连接（基线）         | 中位数 9.0 ms，p95 10.3 ms  |                 |
| `hostlane exec test -- true`（每次新起 CLI 进程） | 中位数 7.9 ms，p95 9.3 ms   |                 |
| 相对复用连接的 OpenSSH 的额外开销                 | socket −4.9 ms，CLI −1.1 ms | < 50 ms         |
| 50 台主机同时连接                                 | 0.3 s 全部连上，RSS 19.4 MB | 50 台           |
| 100 条隧道                                        | 全部 active，RSS 20.7 MB    | 100 条          |
| 隧道吞吐（单个连接）                              | 491 MB/s                    |                 |
| 跑完后的内存（RSS）                               | 21.4 MB                     |                 |
