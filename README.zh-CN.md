<div align="center">
  <h1>Hostlane</h1>
  <p><a href="README.md">English</a> · <b>简体中文</b></p>
  <p>给你自己的 agent 用的 SSH 运行时。让 Codex、Claude Code 或自研 agent 在本机操作远程服务器：执行命令、修改文件、端口映射和管理 web 服务端口，并带有指纹校验、审批和审计。</p>
  <p>不内置 LLM；服务器端不需要安装任何组件，只要有 <code>sshd</code>。</p>
  <p>
    <img alt="Status" src="https://img.shields.io/badge/status-early_development_(M6)-orange" />
    <img alt="Rust" src="https://img.shields.io/badge/Rust-2024-000000?logo=rust&logoColor=white" />
    <img alt="Electron" src="https://img.shields.io/badge/Electron-45-47848F?logo=electron&logoColor=white" />
    <img alt="React" src="https://img.shields.io/badge/React-19-149ECA?logo=react&logoColor=white" />
    <img alt="TypeScript" src="https://img.shields.io/badge/TypeScript-strict-3178C6?logo=typescript&logoColor=white" />
    <img alt="License" src="https://img.shields.io/badge/license-MIT-lightgrey" />
  </p>
  <p>
    <a href="#快速开始">快速开始</a> ·
    <a href="#接入你的-agent">接入你的 agent</a> ·
    <a href="#路线图">路线图</a> ·
    <a href="docs/architecture.md">架构与协议</a> ·
    <a href="#常见问题">常见问题</a>
  </p>
</div>

> [!WARNING]
> Hostlane 仍处于早期开发阶段。目前可用的是 M0–M6：守护进程、CLI、导入 `~/.ssh/config`、SSH 连接池与主机指纹校验、`exec`、会话监视器、审计日志、文件操作（读取、列目录、搜索、带哈希校验的原子修改、上传、下载、撤销），端口管理（端口发现，关联进程、systemd 和 Docker，HTTP/HTTPS 探测和证书有效期，备注，空闲端口查找）和隧道（本地转发和端口冲突检测、保存的隧道随守护进程恢复、断线自动重连、agent 临时隧道到期关闭）和策略（命令风险分级、规则、带 diff 的阻塞式或异步审批、急停），以及 MCP Server、登录时启动的后台服务（launchd 或 systemd）和内置二进制的桌面端安装包。目前还没有签名发布版，见[路线图](#路线图)。

<picture>
  <source media="(prefers-color-scheme: dark)" srcset="docs/screenshots/hosts-dark.png" />
  <img alt="主机页" src="docs/screenshots/hosts-light.png" />
</picture>

<table>
  <tr>
    <td><img alt="接入页（浅色）" src="docs/screenshots/integrations-light.png" /></td>
    <td><img alt="接入页（深色）" src="docs/screenshots/integrations-dark.png" /></td>
  </tr>
</table>

<sub>截图使用的是一份示例 <code>ssh_config</code>，其中的主机都是虚构的。</sub>

## 为什么做它

agent 已经很擅长运维服务器，但周边工具跟不上：Chaterm、Warp 等终端自带 LLM；VS Code Remote-SSH 和 Claude Code 的 SSH 模式会把运行时装到服务器上；现有的 SSH MCP 在安全编辑、长任务、审批和多主机上都不完整。Hostlane 只给**你的** agent 提供“手脚”，“大脑”留在你这边：

- **自带 agent**：Hostlane 没有模型，也不调用任何模型 API。能执行 shell 命令或支持 MCP 的 agent 都能使用。
- **服务器零安装**：只依赖 `sshd`、POSIX shell 和 SFTP。有 `rg`、`tmux`、`docker` 时自动使用，没有时自动降级。
- **默认安全**：首次连接时记录主机指纹，变化时拒绝连接；命令按风险分级，高风险操作等你审批；每次调用都写入本地只追加的审计日志；一键急停可以暂停所有 agent 操作。
- **可靠的文件修改**：打补丁前先校验哈希、备份原文件，再上传临时文件并原子替换，保留权限和属主。
- **面向 web 服务**：查看每台服务器监听了哪些端口、属于哪个进程、systemd 单元或 Docker 容器、能否响应 HTTP，并一键转发到本机。
- **一个守护进程，多个客户端**：常驻的 Rust 守护进程持有 SSH 连接、隧道和审批队列，CLI、MCP 和桌面端共用它；桌面端随时可以关闭。
- **只看不输**：会话监视器是只读终端，实时或回放显示每条命令的输出、退出码和耗时。

## 工作方式

```text
你的 agent ──CLI / MCP──┐
                        ▼
Electron UI ──IPC──► 主进程 ──本地 socket──► hostlane daemon ──SSH/SFTP──► 服务器
                                              ├─ 连接池
                                              ├─ 执行器 / 后台任务
                                              ├─ 文件操作
                                              ├─ 端口映射（隧道）
                                              ├─ 端口发现与服务识别
                                              ├─ 策略引擎 / 审批队列
                                              └─ 审计日志（SQLite）
```

- `hostlane` 是单一二进制，包含 CLI、守护进程（`hostlane daemon …`）和 MCP Server（`hostlane mcp`）。
- 客户端通过当前用户专属、权限为 `0600` 的 socket，以换行分隔的 JSON-RPC 2.0 与守护进程通信。
- 错误带有稳定的 `kind` 和写给 agent 的 `hint`，agent 不用解析自然语言就能决定下一步。

详见[架构与协议](docs/architecture.md)。

## 快速开始

目前还没有发布版，需要从源码构建。

**环境要求**：Rust 1.89+、Node.js 22.12+、npm 10+。

```bash
git clone https://github.com/rzh-98/Hostlane.git && cd Hostlane

# 守护进程 + CLI（测试会同时重新生成桌面端的 TS 协议类型）
cargo build
cargo test --workspace

./target/debug/hostlane daemon start
./target/debug/hostlane hosts
./target/debug/hostlane trust <host>              # 首次连接：核对并信任主机指纹
./target/debug/hostlane exec <host> -- uname -a
./target/debug/hostlane read <host> /etc/hostname
./target/debug/hostlane daemon stop

# 可选：登录时启动守护进程，崩溃后自动重启（launchd / systemd --user）
./target/debug/hostlane daemon install

# 桌面端（开发模式使用 target/debug/hostlane，必要时自动拉起守护进程）
cd apps/desktop
npm install
npm run dev

# 打包桌面端，release 版 hostlane 放在 Resources/bin（未签名）
npm run pack        # 或 npm run dist:mac / dist:linux
```

### CLI

所有命令都支持 `--json`。成功时 stdout 输出结果；失败时退出码为 1，stdout 输出 `{"error":{"kind","message","hint"}}`。

| 命令                                                                               | 作用                                                                     |
| ---------------------------------------------------------------------------------- | ------------------------------------------------------------------------ |
| `hostlane status`                                                                  | 查看守护进程是否在运行，以及版本、PID 和 socket                          |
| `hostlane hosts`                                                                   | 列出 `~/.ssh/config` 中的主机（支持 `Include`、通配符块和首次赋值生效）  |
| `hostlane exec <host> [--cwd DIR] [--env K=V] [--timeout SECS] [--stdin] -- <cmd>` | 在远程主机上执行命令。不加 `--json` 时退出码等于远程退出码，超时为 124   |
| `hostlane output <output_id> [--stream stderr] [--offset N]`                       | 读取被截断的完整输出（负偏移表示从末尾倒数）                             |
| `hostlane read <host> <path> [--offset N] [--limit N]`                             | 输出文本文件；`sha256` 和行范围写到 stderr                               |
| `hostlane ls <host> [path]`                                                        | 列出目录                                                                 |
| `hostlane search <host> <pattern> [path] [--glob G] [-F] [-i]`                     | 搜索文件内容：服务器上有 `rg` 时用 `rg`，否则用 `grep -r`                |
| `hostlane write <host> <path> [--expect SHA256 \| --overwrite] [--mode 0644]`      | 把 stdin 原子写入文件；替换已有文件需要当前哈希或 `--overwrite`          |
| `hostlane edit <host> <path> --old TEXT --new TEXT [--all] [--expect SHA256]`      | 精确替换文本（重复 `--old`/`--new` 可做多处修改）                        |
| `hostlane patch <host> <path> [--expect SHA256]`                                   | 应用 stdin 中的 unified diff                                             |
| `hostlane upload` / `download <host> <from> <to>`                                  | 上传或下载文件（上传同样原子替换并校验）                                 |
| `hostlane diff` / `revert <change_id>`                                             | 查看一次文件修改的 diff，或在文件之后没被改过时撤销它                    |
| `hostlane ports <host> [--no-probe]`                                               | 列出监听的 TCP 端口，以及对应的进程、systemd 单元或容器、HTTP 响应和备注 |
| `hostlane port <host> <port> [--server-name DOMAIN]`                               | 查看单个端口：地址、进程、服务、HTTP 状态、标题和 TLS 证书               |
| `hostlane port-check <host> <port>`                                                | 检查端口是否被占用，占用时给出最近的空闲端口                             |
| `hostlane free-port <host> [--from N] [--to N] [--count N]`                        | 查找空闲端口（默认 8000–9999），有备注的端口视为已预留                   |
| `hostlane port-note <host> <port> [--name N] [--project P] [--note T]`             | 给端口命名并关联项目；不带任何字段时清除备注                             |
| `hostlane tunnel open <host> <port> [--local-port N] [--save] [--ttl SECS]`        | 把服务器端口转发到本机 `127.0.0.1`；自动选空闲端口，冲突时给出建议端口   |
| `hostlane tunnels`                                                                 | 列出隧道、状态、本机地址、连接数和流量                                   |
| `hostlane tunnel close <id>` / `close --session S`                                 | 关闭一条隧道，或关闭某个 agent 会话创建的全部临时隧道                    |
| `hostlane tunnel start` / `update` / `delete <id>`                                 | 重新启动已停止的隧道、修改端口或保存，或删除隧道                         |
| `hostlane trust <host> [--fingerprint SHA256:…]`                                   | 核对并信任主机指纹，写入 Hostlane 自己的 `known_hosts`                   |
| `hostlane connect` / `disconnect <host>`                                           | 建立或断开到主机的连接                                                   |
| `hostlane connections`                                                             | 查看连接状态、延迟、活动通道和主机指纹                                   |
| `hostlane watch [--host H]`                                                        | 实时查看所有 agent 执行的命令和输出（只读）                              |
| `hostlane history`                                                                 | 最近执行的命令                                                           |
| `hostlane audit [--host H] [--client C] [--min-risk L2]`                           | 查看审计日志，含每次调用的风险等级和审批人                               |
| `hostlane approvals [--all]` / `approval <id>`                                     | 列出待审批的调用，或查看某一项的命令或 diff、风险和原因                  |
| `hostlane approve <id> [--scope once\|session\|always]` / `deny <id> [--note T]`   | 审批（只能由人执行；`always` 会添加一条允许规则）                        |
| `hostlane stop [--reason T]` / `resume`                                            | 急停：拒绝 agent 调用、终止其命令并关闭其隧道；`resume` 恢复             |
| `hostlane label <host> [--env prod] [--tag T]… [--clear]`                          | 设置主机的环境和标签，规则和默认策略会用到                               |
| `hostlane policy status` / `add` / `delete` / `check <host> <cmd>`                 | 查看或修改规则，或预先查看命令的风险和策略决定                           |
| `hostlane call <method> '<params>'`                                                | 直接向守护进程发送 JSON-RPC 调用                                         |
| `hostlane mcp`                                                                     | 以 stdio 方式向 agent 提供 MCP 工具；守护进程没运行时自动启动            |
| `hostlane daemon start` / `stop`                                                   | 在后台启动守护进程，或让它退出                                           |
| `hostlane daemon install [--print]` / `uninstall` / `service`                      | 注册为登录时启动的用户级服务（launchd 或 `systemd --user`）、移除或查看  |
| `hostlane daemon run`                                                              | 在前台运行守护进程                                                       |
| `hostlane daemon socket-path`                                                      | 输出客户端使用的 socket 路径                                             |

### 环境变量

| 变量                  | 作用                                                                      |
| --------------------- | ------------------------------------------------------------------------- |
| `HOSTLANE_SOCKET`     | 覆盖 socket 路径                                                          |
| `HOSTLANE_DATA_DIR`   | 覆盖数据目录（日志、数据库）                                              |
| `HOSTLANE_SSH_CONFIG` | 覆盖 `ssh_config` 路径                                                    |
| `HOSTLANE_BIN`        | 桌面端使用的 `hostlane` 程序路径                                          |
| `HOSTLANE_CLIENT`     | 调用来源名称（默认 `cli`，`hostlane mcp` 为 `mcp`），显示在审计和会话页中 |
| `HOSTLANE_SESSION`    | agent 会话 ID，会话页按它分组                                             |
| `HOSTLANE_NOTIFY`     | 设为 `0` 时，守护进程不再自己发送审批的系统通知                           |

不想影响真实环境时，可以把这些变量都指向临时目录：

```bash
export HOSTLANE_SOCKET=/tmp/hl/hostlane.sock HOSTLANE_DATA_DIR=/tmp/hl HOSTLANE_SSH_CONFIG=/tmp/hl/ssh_config
```

## 接入你的 agent

### MCP

`hostlane mcp` 是 stdio MCP Server，在 agent 的配置里添加它即可。桌面端“接入”页会给出填好路径的配置片段。如果 agent 的 `PATH` 里没有 `hostlane`，请使用绝对路径（“接入”页可以把它链接到 `~/.local/bin`）。

**Codex**（`~/.codex/config.toml`）：

```toml
[mcp_servers.hostlane]
command = "hostlane"
args = ["mcp"]
# 等待审批时调用会阻塞，留出足够时间
tool_timeout_sec = 600
```

**Claude Code**：

```bash
claude mcp add --scope user hostlane -- hostlane mcp
```

**Cursor**（`~/.cursor/mcp.json` 或项目中的 `.cursor/mcp.json`）：

```json
{
  "mcpServers": {
    "hostlane": { "command": "hostlane", "args": ["mcp"] }
  }
}
```

工具与 CLI 一一对应：

| 工具                                                                                     | 对应的 CLI                                               |
| ---------------------------------------------------------------------------------------- | -------------------------------------------------------- |
| `hosts`                                                                                  | `hosts`、`connections`                                   |
| `exec`、`read_output`、`exec_history`                                                    | `exec`、`output`、`history`                              |
| `read_file`、`list_dir`、`search_files`                                                  | `read`、`ls`、`search`                                   |
| `write_file`、`edit_file`、`apply_diff`、`upload_file`、`download_file`、`revert_change` | `write`、`edit`、`patch`、`upload`、`download`、`revert` |
| `list_ports`、`inspect_port`、`check_port`、`find_free_port`、`annotate_port`            | `ports`、`port`、`port-check`、`free-port`、`port-note`  |
| `list_tunnels`、`open_tunnel`、`close_tunnel`                                            | `tunnels`、`tunnel open`、`tunnel close`                 |
| `check_policy`、`get_approval`                                                           | `policy check`、`approval`                               |

- 策略、审批和审计与 CLI 完全相同。需要审批的调用会阻塞等待（最长 `approval_timeout_secs`，默认 120 秒）；传 `async: true` 时立即返回 `APPROVAL_REQUIRED` 和 `ticket_id`，审批通过后 agent 带上 `ticket_id` 重试同一调用。
- 错误以工具错误返回，内容为 `{"error": {"kind", "message", "hint", "details"}}`。
- 只能由人执行的操作不对 MCP 开放：审批、规则、信任主机指纹、主机标签和急停后恢复。
- 调用方记为 `mcp`（或 `HOSTLANE_CLIENT`）；每个 MCP Server 进程是一个会话，以 MCP 客户端命名，例如 `codex-mcp-client-4121`。

### 通过 CLI

只能执行 shell 命令的 agent 使用 CLI。把下面这段放进项目的 `AGENTS.md` 或 `CLAUDE.md`：

```markdown
## 远程服务器

通过 `hostlane` CLI 操作服务器，不要直接运行 ssh。

- 先运行 `hostlane --json hosts` 查看可用主机。
- 执行命令：`hostlane --json exec <host> [--cwd DIR] [--timeout SECS] -- <command>`。
- 输出过长会被截断（truncated 为 true），用 `hostlane --json output <output_id>` 读取全文。
- 用 Hostlane 改文件，不要用 sed 或 heredoc：`hostlane --json read <host> <path>` 会返回 `sha256`，把它作为 `--expect` 传给 `edit --old … --new …`、`patch`（stdin 传 unified diff）或 `write`（stdin 传内容）。遇到 FILE_CHANGED 时重新读取文件再修改。
- 搜索用 `hostlane --json search <host> <pattern> [path]`，列目录用 `hostlane --json ls <host> [path]`。
- 每次修改都会返回 `change_id`，用 `hostlane revert <change_id>` 撤销。
- 部署服务前用 `hostlane --json free-port <host>` 或 `port-check <host> <port>` 选端口，不要猜；`hostlane --json ports <host>` 列出端口对应的进程、容器和 HTTP 状态，`port <host> <port>` 查看单个端口。
- 用 `hostlane port-note <host> <port> --name … --project …` 记录新部署服务占用的端口。
- 需要在本机访问服务器上的服务时，用 `hostlane --json tunnel open <host> <port>` 建立隧道，使用返回的 `tunnel.url`；用完后 `hostlane tunnel close <id>`，会话结束时运行 `hostlane tunnel close --session <会话 ID>`。不要绑定 0.0.0.0，除非用户明确要求。
- 命令都加 `--json`；失败时读取 error.kind 和 error.hint 决定下一步。
- 遇到 HOST_KEY_UNKNOWN 或 HOST_KEY_CHANGED 时停下来告诉用户，不要自行信任主机密钥。
- 高风险操作（sudo、重启服务、删除目录等）需要用户在 Hostlane 中审批，命令会阻塞等待（默认 120 秒）。不想阻塞时加 `--async`：返回 APPROVAL_REQUIRED 和 error.details.ticket_id，告诉用户后用 `--ticket <ticket_id>` 重试完全相同的命令。可以先用 `hostlane --json policy check <host> <command>` 查看风险。
- 遇到 POLICY_DENIED 时不要换个写法绕过；按 error.hint 告诉用户。不要运行 `hostlane approve`、`deny`、`resume` 或 `policy add`，这些只能由用户执行。
- 设置 `HOSTLANE_CLIENT=<agent 名称>` 和 `HOSTLANE_SESSION=<会话 ID>`，审计和会话页会按它们区分来源。
```

桌面端的“接入”页也有这段说明，可以一键复制。

## 路线图

| 阶段              | 内容                                                                                      | 状态      |
| ----------------- | ----------------------------------------------------------------------------------------- | --------- |
| **M0** 骨架       | Rust workspace、守护进程、本地 socket JSON-RPC、CLI、读取 `~/.ssh/config`、桌面端页面框架 | ✅ 已完成 |
| **M1** 连接与执行 | `russh` 连接池、主机指纹校验、`exec`、会话监视器、审计日志                                | ✅ 已完成 |
| **M2** 文件       | `read_file`、`list_dir`、`search`、`apply_patch`、`write_file`                            | ✅ 已完成 |
| **M3** 端口       | 端口发现、进程关联、HTTP 探测、端口备注、空闲端口查找                                     | ✅ 已完成 |
| **M4** 隧道       | 本地转发、配置持久化、自动重连、冲突检测、agent 临时隧道                                  | ✅ 已完成 |
| **M5** 策略       | 风险分级、规则配置、阻塞式审批、急停                                                      | ✅ 已完成 |
| **M6** MCP 与分发 | `hostlane mcp`、接入页、用户级服务注册、打包内置二进制                                    | ✅ 已完成 |

完整需求（含编号和优先级）见 [docs/requirements.md](docs/requirements.md)。

## 项目结构

```text
├─ Cargo.toml                  Rust workspace
├─ crates/
│  ├─ hostlane-protocol/       线上传输的类型：JSON-RPC 帧、错误码、各方法的参数和结果
│  ├─ hostlane-core/           守护进程：socket 服务、SSH 连接池、exec、文件（SFTP）、端口、隧道、策略与审批、审计（SQLite）、客户端
│  └─ hostlane-cli/            `hostlane` 二进制：CLI、MCP Server（`mcp`）、`daemon` 子命令和服务注册
├─ apps/desktop/               Electron 桌面端（基于 Desktop-Starter）
│  └─ src/
│     ├─ main/daemon.ts        连接守护进程、按需拉起、转发事件
│     ├─ main/setup.ts         把 CLI 链接到 ~/.local/bin，注册登录服务
│     ├─ shared/protocol/      由 hostlane-protocol 生成的 TS 类型（不要手改）
│     └─ renderer/src/pages/   主机、端口、隧道、会话、审批、审计、接入、设置
└─ docs/                       需求、架构、截图
```

依赖方向：`hostlane-cli → hostlane-core → hostlane-protocol`。桌面端只依赖生成的 TS 类型。

## 开发

```bash
# Rust
cargo fmt --all
cargo clippy --workspace --all-targets
cargo test --workspace

# 桌面端（在 apps/desktop 下）
npm run typecheck
npx prettier --write src
```

- 新增一个 JSON-RPC 方法需要五步，见[新增一个方法](docs/architecture.md#新增一个方法)。
- 桌面端 UI 遵循[设计系统](apps/desktop/docs/design-system.zh-CN.md)；`ui/` 和 `layout/` 中不放业务代码。
- socket 路径规则同时存在于 `crates/hostlane-core/src/paths.rs` 和 `apps/desktop/src/main/paths.ts`，修改时两边保持一致。

## 安全

- 本地 socket 只允许当前用户访问（权限 `0600`，所在目录 `0700`）。
- SSH 私钥、密码和审计数据只保存在本机。凭据不进入渲染进程，也不写入日志。
- 首次连接时必须由人核对并信任主机指纹；指纹变化时拒绝连接，agent 和 `hosts.trust` 都无法绕过。Hostlane 只写自己的 `known_hosts`，不修改 `~/.ssh/known_hosts`。
- 审计日志只追加：数据库触发器禁止修改和删除记录。
- 每个访问服务器的调用在执行前都会分级并按规则检查。桌面端自己的操作总是允许；CLI 按 agent 对待，但在终端中运行时会直接询问你。
- 调用方名称（`HOSTLANE_CLIENT`）由客户端自己声明，没有认证。策略用来防止守规矩的 agent 犯错，不能隔离以你的用户身份运行的代码。
- 认证使用 ssh-agent 或未加密的私钥文件；带口令的私钥请先 `ssh-add`，密码登录暂不支持。
- 渲染进程运行在沙箱中，只能调用白名单内的守护进程方法。
- Hostlane 不收集任何遥测数据，除了你的服务器之外不连接任何地方。

## 常见问题

<details>
<summary>为什么不直接让 agent 运行 <code>ssh</code>？</summary>

能用，但问题不少：每条命令都新建连接；没有主机指纹策略；用 `sed -i` 改文件不留备份；agent 和生产环境上的 `rm -rf` 之间没有任何阻拦；事后也查不到做过什么。Hostlane 为每台主机复用一条连接，并在此基础上提供安全编辑、审批、隧道和审计。

</details>

<details>
<summary>会在服务器上安装东西吗？</summary>

不会。只用 `sshd`、POSIX shell 和 SFTP。服务器上有 `rg`、`tmux`、`docker` 等工具时会使用它们，没有时退回 `grep`、`nohup` 等基础命令。

</details>

<details>
<summary>用的是哪个 LLM？</summary>

不用任何 LLM。Hostlane 是运行时，你正在用的 agent 才是“大脑”。

</details>

<details>
<summary>能在会话终端里手动输入吗？</summary>

不能，这是有意的设计。会话监视器是只读的，这样服务器上发生的所有操作都会经过同一套策略检查和审计。

</details>

<details>
<summary>支持 Kubernetes 和 Windows 吗？</summary>

端口管理支持 systemd、Docker 和 docker-compose，暂不支持 Kubernetes。目前主要支持 macOS 和 Linux；Windows 的命名管道支持计划在 V1 实现。

</details>

## 范围

Hostlane **不包含** LLM、团队协作和云端同步、Kubernetes 支持，也不支持人工在终端中输入。它专注于做一个小巧、安全、可由任何 agent 驱动的本地运行时。

## 许可证

[MIT](LICENSE)
