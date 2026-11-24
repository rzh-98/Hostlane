export type McpClient = 'codex' | 'claude' | 'cursor'

export interface McpSnippet {
  /** Where the snippet goes. */
  where: string
  language: 'toml' | 'shell' | 'json'
  text: string
}

function shellQuote(word: string): string {
  return /^[\w@%+=:,./-]+$/.test(word) ? word : `'${word.replace(/'/g, `'\\''`)}'`
}

/** MCP config for each client, running `<command> mcp` with the given environment. */
export function mcpSnippet(
  client: McpClient,
  command: string,
  env: Record<string, string>
): McpSnippet {
  const entries = Object.entries(env)
  switch (client) {
    case 'codex': {
      const lines = [
        '[mcp_servers.hostlane]',
        `command = ${JSON.stringify(command)}`,
        'args = ["mcp"]',
        '# 等待审批时调用会阻塞，留出足够时间',
        'tool_timeout_sec = 600'
      ]
      if (entries.length > 0) {
        lines.push('', '[mcp_servers.hostlane.env]')
        for (const [name, value] of entries) lines.push(`${name} = ${JSON.stringify(value)}`)
      }
      return { where: '~/.codex/config.toml', language: 'toml', text: lines.join('\n') }
    }
    case 'claude': {
      const flags = entries.map(([name, value]) => `--env ${shellQuote(`${name}=${value}`)} `)
      return {
        where: '在终端中运行',
        language: 'shell',
        text: `claude mcp add --scope user ${flags.join('')}hostlane -- ${shellQuote(command)} mcp`
      }
    }
    case 'cursor': {
      const server: Record<string, unknown> = { command, args: ['mcp'] }
      if (entries.length > 0) server['env'] = env
      return {
        where: '~/.cursor/mcp.json（或项目中的 .cursor/mcp.json）',
        language: 'json',
        text: JSON.stringify({ mcpServers: { hostlane: server } }, null, 2)
      }
    }
  }
}
