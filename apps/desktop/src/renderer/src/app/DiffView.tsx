import { cn } from '@/lib/cn'

function lineClass(line: string): string {
  if (line.startsWith('+++') || line.startsWith('---')) return 'font-semibold text-fg'
  if (line.startsWith('@@')) return 'text-accent-text'
  if (line.startsWith('+')) return 'bg-success-subtle text-success-text'
  if (line.startsWith('-')) return 'bg-danger-subtle text-danger-text'
  return 'text-fg-muted'
}

/** Unified diff lines, colored. Wrap it in a `<pre>`. */
export function DiffLines({ diff }: { diff: string }) {
  const lines = diff.split('\n')
  if (lines[lines.length - 1] === '') lines.pop()
  return lines.map((line, index) => (
    <div key={index} className={cn('px-3 whitespace-pre-wrap break-all', lineClass(line))}>
      {line || ' '}
    </div>
  ))
}
