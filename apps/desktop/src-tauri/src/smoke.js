(() => {
  if (!window.desktop || !document.querySelector('#root > *')) return false
  if (!document.body.textContent.includes('tauri-smoke')) return false
  void Promise.all([
    window.desktop.hostlane.call('hosts.list'),
    window.desktop.hostlane.call('secrets.set')
  ]).then(([hosts, blocked]) => {
    window.__smoke = hosts.ok && hosts.result.hosts.some(host => host.id === 'tauri-smoke') && !blocked.ok && blocked.error.kind === 'INVALID_PARAMS'
  })
  return window.__smoke === true
})()
