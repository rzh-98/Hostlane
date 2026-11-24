# Hostlane 开发说明

- 需求和里程碑以 `docs/requirements.md` 为准，协议和目录约定见 `docs/architecture.md`。
- Rust：改完运行 `cargo fmt --all`、`cargo clippy --workspace --all-targets`、`cargo test --workspace`。
- `apps/desktop/src/shared/protocol/` 由 `cargo test` 通过 ts-rs 生成，不要手改。
- 桌面端：在 `apps/desktop` 下运行 `npm run typecheck` 和 `npx prettier --write src`。UI 遵循 `apps/desktop/docs/design-system.zh-CN.md`；`ui/` 和 `layout/` 中不放业务代码。
- socket 路径规则同时存在于 `crates/hostlane-core/src/paths.rs` 和 `apps/desktop/src/main/paths.ts`，修改时两边保持一致。
- 本地测试不要影响真实环境：把 `HOSTLANE_SOCKET`、`HOSTLANE_DATA_DIR`、`HOSTLANE_SSH_CONFIG` 指向临时目录。
- SSH 凭据不能进入渲染进程，也不能写入日志。
