# ic-canister-storage

`ic-canister-storage` 是一个使用 Rust 2024 编写的 Internet Computer 文件存储 Canister。它负责接收分块上传的
文件，把文件内容保存在 Stable Structures 中，并通过 Candid 和 IC HTTP 接口提供查询、下载和网页访问能力。

项目同时集成了 `ic-canister-kit` 提供的权限、维护模式、操作记录、定时任务和版本化升级框架，适合用作小型静态
资源站点、前端资源托管或其他需要 Canister 内文件存储的项目基础。

## 核心能力

- 分块上传：按 `path` 接收文件块，校验块序号、块大小、文件总大小和 headers，全部上传完成后再发布文件。
- 哈希复用：文件以 SHA-256 hash 标识；相同内容可以复用底层数据，只新增路径和 headers 元数据。
- 哈希校验模式：`hashed = false` 时由 Canister 在上传完成后重新计算 hash；`hashed = true` 时信任调用方提供的
  hash，适合可信上传工具减少重复计算。
- 文件管理：支持查询文件列表、完整下载、按 offset/size 下载、覆盖同路径文件和批量删除路径。
- HTTP 访问：访问 `/` 可查看内置文件列表页，访问文件路径可直接获取内容；大响应通过 IC streaming callback
  分段返回，并携带保存的 headers 和 ETag。
- 权限控制：查询、上传、删除、维护、Record 和 schedule 使用独立权限，可直接授权用户，也可通过角色授权。
- 运维能力：支持维护模式、cycles 查询与充值、Canister 状态查询、操作 Record、定时任务和 PocketIC 回归。
- 升级恢复：堆状态通过 `pre_upgrade`/`post_upgrade` 快照恢复，文件内容保留在固定 MemoryId 的 Stable
  Structures 中。

## 文件上传与访问流程

1. 上传端读取本地文件，计算 SHA-256，并根据 `chunk_size` 拆分数据。
2. 上传端依次调用 `business_upload`，传入文件路径、headers、hash、总大小、块序号和当前块内容。
3. Canister 在 heap 中维护上传进度；所有块到齐后生成文件元数据，并把内容切成最多 2 MiB 的内部数据块写入
   stable memory。
4. Candid 调用方可以通过 `business_files`、`business_download` 或 `business_download_by` 读取文件。
5. 浏览器可以通过 Canister HTTP 地址直接访问文件路径；超过单次 HTTP 响应上限的数据会继续走
   `http_streaming`。

仓库中的 `tests/upload.rs` 是配套上传工具：它比较本地目录与远端文件列表，删除远端多余路径，并只上传新增或
发生变化的文件。默认配置位于文件顶部，执行前应确认 `IDENTITY`、`NETWORK` 和 `ASSETS_DIR`。

## 运行边界

- 单文件最大 256 MiB；单次 `business_upload` 最多提交 64 个分块，分块数据合计不得超过 2 MiB。
- 单次 `business_delete` 最多提交 1000 个路径，路径文本合计不得超过 64 KiB。
- 同时最多保留 16 个未完成上传；已发布资源与未完成上传缓冲区的逻辑总量不得超过 1 GiB。
- 路径最大 1024 bytes，不能包含控制字符、`?` 或 `#`。
- 单文件最多 64 个 Header，Header 总大小不得超过 64 KiB；协议管理头、非法名称以及包含换行的值会被拒绝。
- `business_download` 和单次 `business_download_by` 受直接 query 响应大小限制，大文件应使用 HTTP streaming。
- HTTP Range 当前支持单段 `bytes` 范围；无法满足或格式不合法的范围返回 `416`。

## 存储模型

项目同时使用 Wasm heap 和 stable memory，两者职责不同：

| 存储区域 | 当前内容 | 升级方式 |
| --- | --- | --- |
| Wasm heap | 权限、维护状态、Record、schedule、文件元数据、hash/path 索引和上传中缓冲区 | `pre_upgrade` 序列化到升级专用稳定内存，`post_upgrade` 按状态版本恢复 |
| Stable Structures | 实际文件数据块，类型为 `StableBTreeMap<SliceOfHashDigest, Vec<u8>>` | 升级后原地保留，不进入 heap snapshot |

当前稳定内存分区：

| MemoryId | 用途 |
| --- | --- |
| `0` | 文件内容数据块；key 由块序号和文件 hash 组成 |
| `254` | `ic-canister-kit` 保留的 heap 升级快照区 |

MemoryId、Stable Structure 类型以及 key/value 编码都是持久化协议的一部分。已经使用的 ID 不能复用，也不能在
原状态版本中直接更换底层类型。需要改变持久化布局时，应新增 `src/stable/vNNN/` 版本和显式迁移。

上传中的完整文件缓冲区位于 heap，因此接口中声明的文件大小上限不等于实际可安全用满的容量；实际部署仍受 IC
单消息、Wasm heap、指令、stable memory 写入和 cycles 等限制。

## 主要接口

完整接口以 `sources/source.did` 为准，常用接口包括：

| 分类 | 接口 |
| --- | --- |
| 文件查询 | `business_files`、`business_download`、`business_download_by` |
| 文件修改 | `business_upload`、`business_delete`、`business_hashed_update` |
| HTTP | `http_request`、`http_streaming` |
| 维护 | `pause_query`、`pause_query_reason`、`pause_replace` |
| 权限 | `permission_all`、`permission_query`、`permission_find_by_user`、`permission_update` |
| Record | `record_topics`、`record_find_by_page`、`record_delete` |
| Schedule | `schedule_find`、`schedule_replace`、`schedule_trigger` |
| Canister | `wallet_balance`、`wallet_receive`、`canister_status`、`whoami`、`version` |

## 目录结构

```text
src/
├── business.rs          # 文件业务 Candid 入口
├── common/              # 通用 API、Candid 生成和共享定义
├── explore.rs           # 内置文件列表页面数据
├── http.rs              # IC HTTP 与 streaming callback
├── stable/              # 状态访问、升级流程和版本化业务实现
└── types.rs             # 公共类型 re-export
tests/
├── business.rs          # 文件业务 PocketIC 回归
├── common.rs            # 权限、维护、Record、schedule 回归
├── regressions.rs       # 资源生命周期、HTTP Range 和边界回归
├── service/             # PocketIC Candid 调用 wrapper
├── upgrade.rs           # 旧 Wasm 到当前 Wasm 的升级回归
└── upload.rs            # 本地资源同步工具
web/                     # 编译进 Canister 的文件列表页
sources/source.did       # 从 Rust 接口生成的 Candid
```

## 开发环境

- Rust `1.97.0`，目标 `wasm32-unknown-unknown`
- `dfx`
- `ic-wasm`
- `gzip`

Rust target 和组件已写入 `rust-toolchain.toml`，进入仓库后 Rustup 会自动选择对应工具链。

## 构建与验证

常用检查：

```bash
cargo fmt --all -- --check
cargo clippy
cargo test
cargo build --target wasm32-unknown-unknown --release
```

公开 query/update、参数或返回类型发生变化后，重新生成并检查 Candid：

```bash
cargo test -p storage update_candid -- --ignored --nocapture
```

执行 `dfx.json` 中的完整构建流程：

```bash
dfx build storage
```

该流程会生成 Candid、构建 release Wasm、注入 Candid metadata、shrink，并输出
`sources/source_opt.wasm.gz`。

## 本地部署与资源同步

首次部署到本地网络：

```bash
dfx start --background
dfx deploy --network local storage
```

升级已经运行的 Canister 前，应先进入维护模式，等待定时任务空闲，再部署并退出维护模式。`deploy.sh` 包含当前
本地网络的升级流程，同时会调用上传工具；它会真实修改 Canister 状态，不应作为普通 build/test 命令执行。

同步 `assets/` 目录：

```bash
bash upload.sh
```

`upload.sh` 最终运行 ignored 的 `upload` 测试，会连接 `tests/upload.rs` 中配置的网络和 identity。执行前必须检查
目标网络，避免把本地资源误同步到其他 Canister。

## PocketIC 回归

- `bash tests/test.sh`：复用已有当前 Wasm，执行升级、通用 API、文件业务和资源/HTTP 回归四组 ignored 测试。
- `bash tests/test.sh update`：先运行普通测试和 Clippy，再重新生成 Candid、构建当前 Wasm 并执行完整回归。
- `sources/source_opt_0_0_1.wasm.gz` 应当是真实历史版本。脚本在 fixture 缺失时会复制当前 Wasm 作为占位，这种
  情况只能验证升级流程，不能证明历史数据兼容。

升级兼容需要分别检查：

1. heap snapshot 是否能由新类型反序列化并完成逐版本迁移。
2. Stable Structures 的 MemoryId、类型和编码是否保持不变。
3. `sources/source.did` 的变化是否要求调用方重新生成 Candid bindings。
