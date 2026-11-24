import { useState } from 'react'
import { Check, Copy } from 'lucide-react'
import { Page, PageContent, Toolbar } from '@/layout'
import { Badge, IconButton, Section } from '@/ui'

const cliCommands: [command: string, description: string][] = [
  ['hostlane --json status', '查看守护进程状态'],
  ['hostlane --json hosts', '列出可用主机'],
  ["hostlane --json call <method> '<params>'", '直接调用任意 JSON-RPC 方法'],
  ['hostlane daemon start', '在后台启动守护进程']
]

const agentsSnippet = `## 远程服务器
通过 \`hostlane\` CLI 操作服务器，不要直接运行 ssh。
- 先运行 \`hostlane --json hosts\` 查看可用主机。
- 命令都加 \`--json\`；失败时读取 error.kind 和 error.hint 决定下一步。`

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
