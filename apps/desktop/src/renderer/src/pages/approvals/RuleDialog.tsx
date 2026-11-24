import { useEffect, useState } from 'react'
import {
  Button,
  Dialog,
  DialogBody,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
  Field,
  Input,
  Select
} from '@/ui'
import type { Decision } from '@shared/protocol/Decision'
import type { HostEnv } from '@shared/protocol/HostEnv'
import type { PolicyRuleAddParams } from '@shared/protocol/PolicyRuleAddParams'
import type { Risk } from '@shared/protocol/Risk'
import type { RuleConditions } from '@shared/protocol/RuleConditions'

const ANY = '*'

interface RuleDialogProps {
  open: boolean
  hosts: string[]
  busy: boolean
  onOpenChange: (open: boolean) => void
  onSubmit: (rule: PolicyRuleAddParams) => void
}

const riskOptions = [
  { value: ANY, label: '不限' },
  { value: 'L0', label: 'L0 只读' },
  { value: 'L1', label: 'L1 用户态修改' },
  { value: 'L2', label: 'L2 系统变更' },
  { value: 'L3', label: 'L3 破坏性' }
]

/** A new policy rule. Text conditions are globs: `*` matches any text, `?` one character. */
export function RuleDialog({ open, hosts, busy, onOpenChange, onSubmit }: RuleDialogProps) {
  const [decision, setDecision] = useState<Decision>('approve')
  const [host, setHost] = useState('')
  const [env, setEnv] = useState(ANY)
  const [client, setClient] = useState('')
  const [method, setMethod] = useState('')
  const [command, setCommand] = useState('')
  const [path, setPath] = useState('')
  const [minRisk, setMinRisk] = useState(ANY)
  const [note, setNote] = useState('')

  useEffect(() => {
    if (!open) return
    setDecision('approve')
    setHost('')
    setEnv(ANY)
    setClient('')
    setMethod('')
    setCommand('')
    setPath('')
    setMinRisk(ANY)
    setNote('')
  }, [open])

  const text = (value: string) => value.trim() || undefined
  const when: RuleConditions = {
    host: text(host),
    env: env === ANY ? undefined : (env as HostEnv),
    client: text(client),
    method: text(method),
    command: text(command),
    path: text(path),
    min_risk: minRisk === ANY ? undefined : (minRisk as Risk)
  }
  const empty = Object.values(when).every((value) => value === undefined)

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent size="sm">
        <DialogHeader>
          <DialogTitle>添加规则</DialogTitle>
          <DialogDescription>
            所有填写的条件都满足时规则生效；留空表示不限。文本条件支持通配符 * 和 ?。
          </DialogDescription>
        </DialogHeader>
        <form
          onSubmit={(event) => {
            event.preventDefault()
            onSubmit({ decision, when, note: text(note) })
          }}
        >
          <DialogBody>
            <div className="grid grid-cols-2 gap-3">
              <Field label="决定">
                <Select
                  value={decision}
                  onValueChange={(value) => setDecision(value as Decision)}
                  options={[
                    { value: 'allow', label: '允许' },
                    { value: 'approve', label: '需要审批' },
                    { value: 'deny', label: '拒绝' }
                  ]}
                />
              </Field>
              <Field label="风险不低于">
                <Select value={minRisk} onValueChange={setMinRisk} options={riskOptions} />
              </Field>
            </div>
            <div className="grid grid-cols-2 gap-3">
              <Field label="主机">
                <Input
                  list="rule-hosts"
                  placeholder="例如 web-*"
                  value={host}
                  onChange={(e) => setHost(e.target.value)}
                />
                <datalist id="rule-hosts">
                  {hosts.map((h) => (
                    <option key={h} value={h} />
                  ))}
                </datalist>
              </Field>
              <Field label="环境">
                <Select
                  value={env}
                  onValueChange={setEnv}
                  options={[
                    { value: ANY, label: '不限' },
                    { value: 'dev', label: 'dev' },
                    { value: 'staging', label: 'staging' },
                    { value: 'prod', label: 'prod' }
                  ]}
                />
              </Field>
            </div>
            <div className="grid grid-cols-2 gap-3">
              <Field label="来源" description="agent 的 HOSTLANE_CLIENT">
                <Input
                  placeholder="例如 claude-code"
                  value={client}
                  onChange={(e) => setClient(e.target.value)}
                />
              </Field>
              <Field label="方法">
                <Input
                  placeholder="例如 files.*"
                  value={method}
                  onChange={(e) => setMethod(e.target.value)}
                />
              </Field>
            </div>
            <Field label="命令" description="匹配整条命令行">
              <Input
                className="font-mono"
                placeholder="例如 systemctl restart *"
                value={command}
                onChange={(e) => setCommand(e.target.value)}
              />
            </Field>
            <Field label="路径" description="匹配文件操作的目标，以及命令读写的路径">
              <Input
                className="font-mono"
                placeholder="例如 /etc/*"
                value={path}
                onChange={(e) => setPath(e.target.value)}
              />
            </Field>
            <Field label="备注">
              <Input maxLength={200} value={note} onChange={(e) => setNote(e.target.value)} />
            </Field>
          </DialogBody>
          <DialogFooter>
            <Button type="button" onClick={() => onOpenChange(false)}>
              取消
            </Button>
            <Button
              type="submit"
              variant="primary"
              disabled={busy || (empty && decision !== 'approve')}
              title={empty && decision !== 'approve' ? '至少填写一个条件' : undefined}
            >
              添加
            </Button>
          </DialogFooter>
        </form>
      </DialogContent>
    </Dialog>
  )
}
