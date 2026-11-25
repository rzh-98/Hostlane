import {
  Button,
  Dialog,
  DialogBody,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle
} from '@/ui'
import type { SecretKind } from '@shared/protocol/SecretKind'

export interface SecretTarget {
  host: string
  kind: SecretKind
}

export function SecretDialog({
  target,
  onClose
}: {
  target: SecretTarget | null
  onClose: () => void
}) {
  const host = target?.host ?? ''
  const simpleAlias = /^[A-Za-z0-9_][A-Za-z0-9_.-]*$/.test(host)
  const password = target?.kind === 'password'
  return (
    <Dialog open={target !== null} onOpenChange={(open) => !open && onClose()}>
      <DialogContent size="sm">
        <DialogHeader>
          <DialogTitle>
            为 {host} 保存{password ? '登录密码' : '密钥口令'}
          </DialogTitle>
          <DialogDescription>
            在本机终端运行命令，按不回显的提示输入。桌面端不会接收凭据。
          </DialogDescription>
        </DialogHeader>
        <DialogBody>
          <code className="selectable break-all">
            hostlane secret set --{password ? 'password' : 'passphrase'} --{' '}
            {simpleAlias ? host : 'HOST_ALIAS'}
          </code>
          {!simpleAlias && <p>请将 HOST_ALIAS 替换为上方的主机名，按当前 shell 的规则引用。</p>}
          {!password && (
            <p>默认使用第一个加密的密钥；要指定其他密钥，在分隔符 -- 前加入 --key 和密钥路径。</p>
          )}
        </DialogBody>
        <DialogFooter>
          <Button onClick={onClose}>关闭并刷新</Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  )
}
