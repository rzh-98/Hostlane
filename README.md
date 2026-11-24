# Hostlane

让你自己的 agent 在本机安全地操作远程服务器：连接、执行、读写文件、端口映射和服务器端口管理。Hostlane 不内置 LLM，服务器端也不需要安装任何组件，只需要 `sshd`。

- 需求：[docs/requirements.md](docs/requirements.md)
- 架构与协议：[docs/architecture.md](docs/architecture.md)
- 桌面端设计系统：[apps/desktop/docs/design-system.zh-CN.md](apps/desktop/docs/design-system.zh-CN.md)

## 当前进度

M0 骨架已完成：

- `hostlane` 单一二进制：常驻守护进程（`hostlane daemon …`）+ CLI。
- 本地 socket JSON-RPC（权限 0600），已实现 `daemon.status`、`daemon.shutdown`、`hosts.list`。
- 按 OpenSSH 规则读取 `~/.ssh/config`，支持 `Include`、通配符块和首次赋值生效。
- 桌面端会自动拉起守护进程；主机页显示真实数据，其他页面列出各自的计划需求。

后续里程碑见 [需求文档第 7 节](docs/requirements.md#7-迭代计划)。

## 开发

环境要求：Rust 1.85+、Node.js 22.12+、npm 10+。

```bash
# Rust：构建、测试（测试会同时重新生成桌面端的 TS 协议类型）
cargo build
cargo test --workspace

# CLI
./target/debug/hostlane daemon start
./target/debug/hostlane hosts
./target/debug/hostlane --json status
./target/debug/hostlane daemon stop

# 桌面端（开发模式会使用 target/debug/hostlane）
cd apps/desktop
npm install
npm run dev
```

### 环境变量

| 变量 | 作用 |
| --- | --- |
| `HOSTLANE_SOCKET` | 覆盖 socket 路径 |
| `HOSTLANE_DATA_DIR` | 覆盖数据目录（日志、数据库） |
| `HOSTLANE_SSH_CONFIG` | 覆盖 ssh_config 路径，便于测试 |
| `HOSTLANE_BIN` | 桌面端使用的 `hostlane` 程序路径 |

不想影响真实环境时，可以把这些变量都指向临时目录：

```bash
export HOSTLANE_SOCKET=/tmp/hl/hostlane.sock HOSTLANE_DATA_DIR=/tmp/hl HOSTLANE_SSH_CONFIG=/tmp/hl/ssh_config
```
