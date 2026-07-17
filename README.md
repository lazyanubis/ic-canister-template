# ic-canister-assets

`ic-canister-assets` 是一个使用 Rust 实现的 Internet Computer 静态资源 Canister。它既可以作为轻量级
资源托管服务使用，也可以作为带有权限、维护模式、升级迁移和操作审计能力的 Canister 项目模板。

## 核心能力

- 通过 Candid 接口分块上传、查询、下载和删除文件，并使用 SHA-256 内容哈希复用相同资源数据。
- 通过 Canister HTTP query 对外提供静态资源，支持自定义响应头、ETag 和大文件 streaming callback。
- 根路径提供内置资源浏览页面，便于查看当前 Canister 保存的文件及其大小、时间和哈希信息。
- 提供角色/用户权限、维护模式、cycles 收款、定时任务和操作 Record 等通用管理能力。
- 使用版本化 `State` 管理堆状态，并在 Canister 升级时通过稳定内存快照完成恢复和逐版本迁移。
- 使用 PocketIC 覆盖公共接口、资源业务和升级流程，Candid 接口保存在 `sources/source.did`。

## 技术与目录

- `src/business.rs`：资源上传、查询、下载和删除入口。
- `src/http.rs`、`src/explore.rs`：HTTP 静态资源响应、流式传输和内置浏览页面。
- `src/common/`：权限、维护模式、Record、schedule、cycles 和 Candid 等通用接口。
- `src/stable/`：状态访问、版本化结构及升级迁移。
- `assets/`：本地待同步资源；`tests/upload.rs` 和 `upload.sh` 提供上传工具。
- `tests/`：PocketIC 集成测试和升级回归。
- 项目使用 Rust 2024、`ic-cdk`、`ic-canister-kit` 和 PocketIC，构建目标为
  `wasm32-unknown-unknown`；`dfx.json` 负责生成 Candid metadata、压缩并输出部署 Wasm。

## 存储边界

- 日常运行状态仅保存在堆内存中；理论最大堆内存为 **4 GiB**。
- 稳定内存不作为日常持久化存储，只在升级前保存状态快照、升级后恢复快照。
- 这是示例模板，不适合存放大量数据。业务开发者应根据自身数据形态主动控制状态规模，避免接近
  4 GiB 堆内存上限后再进行升级。
- 模板不对 Record 施加通用的条目数、字节数或截断策略，以免丢失业务必须保留的日志；每项业务负责
  在入口处约束自己写入状态或日志的数据规模。

## 状态与升级

- `State::V*`、`StateUpgrade` 与版本化的 Init/Upgrade 参数共同约束状态迁移；`version()` 只表示
  存储状态版本。
- 升级前必须进入维护状态且定时任务空闲；恢复后完成状态迁移和 schedule 校验，退出维护状态时再启动定时任务。

## 业务运行约束

- 单文件最大 256 MiB；单次 `business_upload` 最多提交 64 个分块且分块数据合计不超过 2 MiB。
- 单次删除最多提交 1000 个路径，路径文本合计不超过 64 KiB。
- 同时最多保留 16 个未完成上传；已发布资源与未完成上传数据合计不得超过 512 MiB，文件路径、Header、
  hash 反向索引和上传分块标记等逻辑元数据合计不得超过 64 MiB。
- 最多发布 10000 个文件。
- 路径最大 1024 bytes；单文件最多 64 个 Header，Header 总大小最大 64 KiB。协议管理头、非法名称及包含换行的值会被拒绝。
- `business_download` 和单次 `business_download_by` 受直接 query 响应大小限制；大文件应使用 HTTP streaming。
- 管理员可以主动移除全部权限；这是运营决策，不由模板强制阻止。
- Record 同时服务于操作历史和已提交状态变更的安全审计。trap 会回滚状态和 Record，符合原子性要求。
- 日志归档采用“分页 query -> 外部保存 -> 按 RecordId 批量删除”；删除不存在的 ID 返回实际删除数量，
  调用方可安全重试。
- `schedule_trigger` 仅允许在非维护模式下运行。
- `tests/test.sh` 无参数时复用已有 Wasm 运行快速回归；传入 `update` 时才重新构建当前 Wasm。
- 升级回归依赖真实历史产物 `sources/source_opt_0_0_1.wasm.gz`；缺失时测试会失败，不会用当前 Wasm
  冒充旧版本。
