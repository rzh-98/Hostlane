# 架构与协议

## 目录结构

```text
Hostlane/
├─ Cargo.toml                    Rust workspace
├─ .cargo/config.toml            ts-rs 的类型导出目录
├─ crates/
│  ├─ hostlane-protocol/         线上传输的类型：JSON-RPC 帧、错误码、各方法的参数和结果
│  ├─ hostlane-core/             守护进程：socket 服务、方法处理、ssh_config 解析、客户端
│  └─ hostlane-cli/              `hostlane` 二进制：CLI + `daemon` 子命令（之后加 `mcp`）
├─ apps/desktop/                 Electron 桌面端（基于 Desktop-Starter）
│  └─ src/
│     ├─ main/daemon.ts          连接守护进程、按需拉起
│     ├─ main/paths.ts           socket 路径规则（与 hostlane-core/src/paths.rs 保持一致）
│     ├─ shared/ipc.ts           IPC 通道、渲染进程可调用的方法白名单
│     ├─ shared/protocol/        由 ts-rs 生成，不要手改
│     └─ renderer/src/app/daemon.ts  守护进程状态 store 与调用封装
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

- 守护进程是唯一持有 SSH 连接、隧道和审批队列的进程。
- `hostlane daemon start` 会以独立进程组在后台启动 `hostlane daemon run`，日志写入 `<数据目录>/daemon.log`，等到 socket 能响应后才返回。
- 桌面端启动时，如果守护进程没有运行，会调用 `hostlane daemon start`。关闭桌面端不会停止守护进程。
- M6 会把守护进程注册为用户级服务（launchd、systemd --user 或登录启动项），由系统负责拉起。

## 路径

| 平台 | 数据目录 | Socket |
| --- | --- | --- |
| macOS | `~/Library/Application Support/Hostlane` | `<数据目录>/hostlane.sock` |
| Linux | `$XDG_STATE_HOME/hostlane`，默认为 `~/.local/state/hostlane` | `$XDG_RUNTIME_DIR/hostlane/hostlane.sock`，没有时使用 `<数据目录>/hostlane.sock` |
| Windows | `%APPDATA%\Hostlane` | 命名管道（V1） |

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
- 协议发生不兼容的变更时，递增 `PROTOCOL_VERSION`，`daemon.status` 会返回该版本号。

### 已实现的方法

| 方法 | 参数 | 结果 |
| --- | --- | --- |
| `daemon.status` | 无 | `DaemonStatus` |
| `daemon.shutdown` | 无 | `null` |
| `hosts.list` | 无 | `HostsList` |

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
