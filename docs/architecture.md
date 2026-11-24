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
│  │     ├─ handlers.rs             方法分发；需要审计的方法在这里写审计日志
│  │     ├─ state.rs                守护进程状态（`Daemon`）、主机解析
│  │     ├─ ssh_config.rs           解析 ~/.ssh/config
│  │     ├─ known_hosts.rs          解析和写入 known_hosts、计算指纹
│  │     ├─ ssh.rs                  russh 客户端：指纹校验、认证
│  │     ├─ pool.rs                 连接池：每台主机一条连接、ProxyJump、保活和空闲回收
│  │     ├─ exec.rs                 exec.run / output.read：超时、截断、输出落盘
│  │     ├─ store.rs                SQLite：审计日志（只追加）和命令记录
│  │     └─ events.rs               事件总线（broadcast）
│  └─ hostlane-cli/              `hostlane` 二进制：CLI + `daemon` 子命令（之后加 `mcp`）
├─ apps/desktop/                 Electron 桌面端（基于 Desktop-Starter）
│  └─ src/
│     ├─ main/daemon.ts          连接守护进程、按需拉起、保持事件订阅并转发给渲染进程
│     ├─ main/paths.ts           socket 路径规则（与 hostlane-core/src/paths.rs 保持一致）
│     ├─ shared/ipc.ts           IPC 通道、渲染进程可调用的方法白名单
│     ├─ shared/protocol/        由 ts-rs 生成，不要手改
│     └─ renderer/src/app/       守护进程状态、事件、连接状态等跨页面的 store
└─ docs/
```

依赖方向：`hostlane-cli → hostlane-core → hostlane-protocol`。桌面端只依赖由 `hostlane-protocol` 生成的 TS 类型，不直接依赖 Rust 代码。

## 进程模型

```text
agent ──shell──► hostlane（CLI，每次调用一个进程）──┐
agent ──stdio──► hostlane mcp（M6）─────────────────┤ 本地 socket
Electron 主进程 ◄─IPC─ 渲染进程（沙箱）─────────────┤ JSON-RPC
                                                    ▼
                                     hostlane daemon run（常驻）
```

- 守护进程是唯一持有 SSH 连接、隧道和审批队列的进程。每台主机复用一条 SSH 连接，每条连接最多同时打开 8 个 channel；空闲 10 分钟后断开，每 30 秒 ping 一次以测量延迟和发现断线。
- SSH 凭据只在守护进程中使用：先用 ssh-agent（`IdentityFile` 对应的密钥优先），再试未加密的私钥文件。带口令的私钥和密码登录暂不支持（H2），请先 `ssh-add`。
- `hostlane daemon start` 会以独立进程组在后台启动 `hostlane daemon run`，日志写入 `<数据目录>/daemon.log`，等到 socket 能响应后才返回。
- 桌面端启动时，如果守护进程没有运行，会调用 `hostlane daemon start`。关闭桌面端不会停止守护进程。
- M6 会把守护进程注册为用户级服务（launchd、systemd --user 或登录启动项），由系统负责拉起。

## 路径

| 平台    | 数据目录                                                     | Socket                                                                           |
| ------- | ------------------------------------------------------------ | -------------------------------------------------------------------------------- |
| macOS   | `~/Library/Application Support/Hostlane`                     | `<数据目录>/hostlane.sock`                                                       |
| Linux   | `$XDG_STATE_HOME/hostlane`，默认为 `~/.local/state/hostlane` | `$XDG_RUNTIME_DIR/hostlane/hostlane.sock`，没有时使用 `<数据目录>/hostlane.sock` |
| Windows | `%APPDATA%\Hostlane`                                         | 命名管道（V1）                                                                   |

数据目录中的文件：

| 文件                                         | 内容                                                                                                                      |
| -------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------- |
| `hostlane.db`                                | SQLite（WAL）：`calls` 表是审计日志，由触发器禁止 UPDATE 和 DELETE；`execs` 表是会话监视器使用的命令记录                  |
| `outputs/<id>.stdout`、`outputs/<id>.stderr` | 命令的完整输出，权限 0600；每个流最多保存 64 MiB，保留 30 天                                                              |
| `known_hosts`                                | Hostlane 信任的主机密钥。Hostlane 也会读取 ssh_config 同目录下的 `known_hosts`（或 `UserKnownHostsFile`），但只写这个文件 |
| `daemon.log`                                 | `hostlane daemon start` 启动时的日志                                                                                      |

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

CLI 从环境变量 `HOSTLANE_CLIENT`（默认 `cli`）和 `HOSTLANE_SESSION` 读取；桌面端固定为 `desktop`。

### 事件订阅

`events.subscribe` 必须是连接上的唯一请求。守护进程先返回 `{"result":{"subscribed":true}}`，之后持续推送通知，直到客户端断开：

```json
← {"jsonrpc":"2.0","method":"event","params":{"type":"exec_output","id":"ex-…","stream":"stdout","data":"…"}}
```

事件类型（`Event`）：`exec_started`、`exec_output`、`exec_finished`、`connection`（连接状态变化），以及 `lagged`（订阅方处理太慢，丢了 `skipped` 条事件，应重新拉取状态）。每条命令的每个流最多推送 1 MiB，完整内容用 `output.read` 读取。

### 已实现的方法

| 方法               | 参数               | 结果                           | 审计 |
| ------------------ | ------------------ | ------------------------------ | ---- |
| `daemon.status`    | 无                 | `DaemonStatus`                 |      |
| `daemon.shutdown`  | 无                 | `null`                         |      |
| `hosts.list`       | 无                 | `HostsList`                    |      |
| `hosts.connect`    | `HostParams`       | `Connection`                   | ✓    |
| `hosts.disconnect` | `HostParams`       | `{"disconnected":bool}`        | ✓    |
| `hosts.trust`      | `TrustParams`      | `TrustResult`                  | ✓    |
| `connections.list` | 无                 | `ConnectionsList`              |      |
| `exec.run`         | `ExecParams`       | `ExecResult`                   | ✓    |
| `exec.list`        | `ExecListParams`   | `ExecList`                     |      |
| `output.read`      | `OutputReadParams` | `OutputChunk`                  |      |
| `audit.list`       | `AuditListParams`  | `AuditList`                    |      |
| `events.subscribe` | 无                 | `Subscribed`，之后推送 `Event` |      |

`exec.run` 的要点：

- `cwd` 和 `env` 会被拼成 `cd '…' || exit 1; export NAME='…'; <command>`，命令由远程用户的登录 shell 执行。
- 超时（默认 120 秒，最长 6 小时）后先发 TERM，2 秒后关闭 channel，状态为 `timed_out`。
- 默认每个流最多返回 16 KiB（`max_output_bytes`，256 B 到 1 MiB），超出时保留头尾并在中间注明省略了多少字节，同时 `truncated` 为 true；完整输出用 `output.read` 按偏移读取，负偏移表示从末尾倒数。
- 主机指纹未知或变化时直接失败（`HOST_KEY_UNKNOWN` / `HOST_KEY_CHANGED`），不会自动信任。`hosts.trust` 要求传入与服务器当前密钥一致的指纹；指纹变化时 `hosts.trust` 同样拒绝，必须由人删除旧记录。
- 审计日志记录参数和结果摘要（不含 stdout/stderr），`stdin` 只记录字节数。守护进程重启时，上次未结束的命令会被标为 `interrupted`。

### 错误类型

| kind                                                                                   | code            | 含义                                                |
| -------------------------------------------------------------------------------------- | --------------- | --------------------------------------------------- |
| `PARSE_ERROR` / `INVALID_REQUEST` / `METHOD_NOT_FOUND` / `INVALID_PARAMS` / `INTERNAL` | -32700 … -32603 | 标准 JSON-RPC 错误                                  |
| `NOT_IMPLEMENTED`                                                                      | -32000          | 方法已规划但尚未实现                                |
| `HOST_NOT_FOUND`                                                                       | -32001          | 主机不在 ssh_config 中；hint 列出已知主机           |
| `HOST_UNREACHABLE`                                                                     | -32002          | TCP 连接或 SSH 握手失败、超时                       |
| `APPROVAL_REQUIRED` / `POLICY_DENIED`                                                  | -32003 / -32004 | 审批与策略（M5）                                    |
| `FILE_CHANGED`                                                                         | -32005          | 文件已被修改（M2）                                  |
| `HOST_KEY_UNKNOWN`                                                                     | -32006          | 主机密钥尚未信任                                    |
| `HOST_KEY_CHANGED`                                                                     | -32007          | 主机密钥与记录不一致，或已被吊销                    |
| `AUTH_FAILED`                                                                          | -32008          | 所有认证方式都失败                                  |
| `CONNECTION_LOST`                                                                      | -32009          | 命令执行中连接断开；部分输出可用 `output.read` 读取 |
| `OUTPUT_NOT_FOUND`                                                                     | -32010          | `output_id` 不存在或已过期                          |

### 新增一个方法

1. 在 `hostlane-protocol` 中定义参数和结果类型（加上 `#[derive(TS)] #[ts(export)]`），并把方法名加入 `method` 模块。
2. 在 `hostlane-core/src/handlers.rs` 的 `dispatch` 中实现。
3. 在 CLI 中加对应的子命令（支持 `--json`）。
4. 如果渲染进程需要调用，把方法名加入 `apps/desktop/src/shared/ipc.ts` 的 `RENDERER_METHODS`。
5. 运行 `cargo test --workspace` 重新生成 TS 类型。

## CLI 约定

- 所有命令都支持 `--json`。成功时 stdout 输出结果；失败时退出码为 1，stdout 输出 `{"error":{"kind","message","hint"}}`。
- 不加 `--json` 时，输出给人看的文本，错误写到 stderr。
- `DAEMON_NOT_RUNNING` 是 CLI 自己产生的错误，表示无法连接 socket。
- `hostlane exec` 不加 `--json` 时，退出码等于远程命令的退出码，超时为 124；加 `--json` 时只要命令开始执行就返回 0，结果看 `status` 和 `exit_code`。
