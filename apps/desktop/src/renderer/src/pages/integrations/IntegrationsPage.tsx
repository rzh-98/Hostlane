import { useState } from 'react'
import { Check, Copy } from 'lucide-react'
import { Page, PageContent, Toolbar } from '@/layout'
import { Badge, IconButton, Section } from '@/ui'

const cliCommands: [command: string, description: string][] = [
  ['hostlane --json hosts', '列出可用主机'],
  ['hostlane --json exec <host> -- <command>', '执行命令，返回退出码、stdout、stderr 和耗时'],
  ['hostlane --json output <output_id>', '读取被截断的完整输出'],
  ['hostlane --json read <host> <path>', '读取文件，返回内容和 sha256'],
  [
    'hostlane --json edit <host> <path> --old … --new … --expect <sha256>',
    '校验哈希后原子修改文件'
  ],
  ['hostlane --json search <host> <pattern> [path]', '用 rg（没有时用 grep）搜索文件内容'],
  ['hostlane revert <change_id>', '撤销一次文件修改'],
  ['hostlane --json ports <host>', '列出监听端口、进程、服务和 HTTP 状态'],
  ['hostlane --json port-check <host> <port>', '检查端口是否被占用，占用时给出建议端口'],
  ['hostlane --json free-port <host> [--from N --to N]', '查找空闲端口（有备注的端口视为已预留）'],
  ['hostlane trust <host>', '核对并信任主机指纹（需要人确认）'],
  ['hostlane watch', '在终端里实时查看所有命令和输出'],
  ["hostlane --json call <method> '<params>'", '直接调用任意 JSON-RPC 方法'],
  ['hostlane daemon start', '在后台启动守护进程']
]

const agentsSnippet = `## 远程服务器
通过 \`hostlane\` CLI 操作服务器，不要直接运行 ssh。
- 先运行 \`hostlane --json hosts\` 查看可用主机。
- 执行命令：\`hostlane --json exec <host> [--cwd DIR] [--timeout SECS] -- <command>\`。
- 输出过长会被截断（truncated 为 true），用 \`hostlane --json output <output_id>\` 读取全文。
- 用 Hostlane 改文件，不要用 sed 或 heredoc：\`hostlane --json read <host> <path>\` 会返回 sha256，把它作为 \`--expect\` 传给 \`edit --old … --new …\`、\`patch\`（stdin 传 unified diff）或 \`write\`（stdin 传内容）。遇到 FILE_CHANGED 时重新读取文件再修改。
- 搜索用 \`hostlane --json search <host> <pattern> [path]\`，列目录用 \`hostlane --json ls <host> [path]\`。
- 每次修改都会返回 change_id，用 \`hostlane revert <change_id>\` 撤销。
- 部署服务前用 \`hostlane --json free-port <host>\` 或 \`port-check <host> <port>\` 选端口，不要猜；\`hostlane --json ports <host>\` 列出端口对应的进程、容器和 HTTP 状态，\`port <host> <port>\` 查看单个端口。
- 用 \`hostlane port-note <host> <port> --name … --project …\` 记录新部署服务占用的端口。
- 命令都加 \`--json\`；失败时读取 error.kind 和 error.hint 决定下一步。
- 遇到 HOST_KEY_UNKNOWN 或 HOST_KEY_CHANGED 时停下来告诉用户，不要自行信任主机密钥。
- 设置 \`HOSTLANE_CLIENT=<agent 名称>\` 和 \`HOSTLANE_SESSION=<会话 ID>\`，审计和会话页会按它们区分来源。`

export function IntegrationsPage() {
  return (
    <Page>
      <Toolbar title="接入" />
      <PageContent width="narrow">
        <Section
          title="CLI"
          description="任何能执行 shell 命令的 agent 都可以直接使用。加上 --json 后，输出和错误都是结构化数据。"
          divided
        >
          {cliCommands.map(([command, description]) => (
            <div key={command} className="flex items-center gap-3 py-2">
              <div className="flex min-w-0 flex-1 flex-col">
                <code className="selectable truncate font-mono text-sm text-fg">{command}</code>
                <span className="text-sm text-fg-muted">{description}</span>
              </div>
              <CopyButton text={command} />
            </div>
          ))}
        </Section>

        <Section
          title="写进 AGENTS.md"
          description="把这段说明放进 agent 的项目指令，让它通过 Hostlane 操作服务器。"
          actions={<CopyButton text={agentsSnippet} />}
        >
          <pre className="selectable overflow-x-auto rounded-md border border-border bg-bg-subtle p-3 font-mono text-sm whitespace-pre-wrap text-fg-muted">
            {agentsSnippet}
          </pre>
        </Section>

        <Section
          title={
            <span className="inline-flex items-center gap-2">
              MCP <Badge>M6</Badge>
            </span>
          }
          description="hostlane mcp 会以 stdio 方式提供与 CLI 一一对应的工具，届时这里会给出 Codex、Claude Code 和 Cursor 的配置片段。"
        />
      </PageContent>
    </Page>
  )
}

function CopyButton({ text }: { text: string }) {
  const [copied, setCopied] = useState(false)
  return (
    <IconButton
      label={copied ? '已复制' : '复制'}
      icon={copied ? <Check /> : <Copy />}
      onClick={() => {
        void navigator.clipboard.writeText(text).then(() => {
          setCopied(true)
          window.setTimeout(() => setCopied(false), 1500)
        })
      }}
    />
  )
}
