# 任务产物存储

[English](task-artifacts.md)

大型任务输入和检查点存放在内容寻址的库存储中，而不是 `task_items.payload`。
本页说明存储布局以及在两个方向移动终端数据的运维工具。

## 存储内容

产物引用是 `context69.task_item_artifacts` 中的一行，包含条目、一种类型、一种
编码和一个存储对象 id：

| 类型 | 编码 | 它替代的负载字段 |
| --- | --- | --- |
| `download` | `raw-v1` | `download_artifact.content_base64` |
| `sections` | `ingest-sections-json-v1` | `section_payload` |

注册表 `context69.library_storage_objects` 仍是分组、SHA-256、大小、后端和对象
键的权威来源。产物行不复制这些值，分组始终派生自条目所属的任务。

任务流水线先读内联数据，再回退到已验证的产物；读不回来的引用是错误，而不是
“没有数据”。重跑和重试复制引用而不是重新上传，文件、任务输入和产物引用共同
保护同一个对象。

存储释放沿用现有机制：删除或改指引用会在
`library_storage_object_cleanup` 中排入旧对象清理，暂存对象保留 24 小时宽限，
被库文件、任务输入或其他条目共享的对象会保留到最后一个引用消失。

## 运维工具

经过审计的运维入口以 `context69 task-artifacts` 形式随应用镜像一起发布。它在读取
配置、数据库准入、迁移、工作线程和信号处理之前就被分发，没有 HTTP 或 UI 入口，
也绝不执行数据库迁移。数据库 URL 只从 `DATABASE_URL` 或仓库 `.env` 读取，绝不接受
命令行参数。`task_artifact_backfill` 是同一实现作为独立开发二进制的入口，可用
`cargo build --bin task_artifact_backfill` 构建。

两个方向默认都只是预览，只有加 `--apply` 才写入。预览使用不执行迁移的运维连接
池，其 PostgreSQL 会话以 `default_transaction_read_only=on` 启动，并且不读取对象
内容。

开发环境可用相同参数运行独立二进制：
`cargo run --bin task_artifact_backfill -- --help`。

### 正向：内联转存储

```sh
context69 task-artifacts                          # 预览
context69 task-artifacts --apply --acknowledge-reader-upgrade
```

正向写入要求产物关系存在，且迁移 `20261019104722` 已按二进制中编译的校验和
应用。它一次只处理一个字段：经内容寻址存储暂存、回读、校验注册表身份，然后在
同一事务中提交引用并移除内联字段。只移除目标字段，包括 `indexing_checkpoint`
在内的其他负载键全部保留。

移除内联字节意味着不含产物读取逻辑的版本无法读取这些条目，因此确认文本说明
必须持续部署产物感知版本。重复运行会报告 `already_externalized`，不做任何更改。

### 反向：存储回内联

```sh
context69 task-artifacts --restore-inline          # 恢复预览
context69 task-artifacts --restore-inline --apply --acknowledge-inline-growth
```

恢复模式扫描现有引用，经校验读取器读取对象（注册表中的大小、摘要、后端和
编码），并且只恢复当前缺失的字段。`download` 字段会重新编码为 Base64，要求既有
`download_artifact` 元数据存在且声明的 SHA-256 等于产物摘要；不会凭空生成源
URL、文件名或其他身份信息。`sections` 字段必须能解析为现有的 ingest 分节数组，
负载其他部分保持不变。

恢复语句先锁父任务再锁条目，重新检查终端条目与父任务、无租约、分组、确切引用
身份和缺失字段，然后在同一语句中写入字段并只删除该引用。期间出现的新认领、引用
变化、分组变化或新出现的内联值都会导致零变更并报告跳过。已存在的内联数据绝不
会被覆盖。

恢复内联数据会使 PostgreSQL 增长。报告统计恢复的编码字节数，并在分配 Base64
文本之前检查字节上限和总预算。报告还给出带 `point_in_time` 标记的剩余引用计数：
在线应用会在查询之后创建新引用，所以零只是快照，不是回滚证明；恢复模式也从不会
回滚 schema 或迁移账本。

### 限制、上限与跳过

两个方向共享同一组边界：

- `--kind download|sections|all`（默认 `all`）；
- 可重复的 `--item-id` 用于有界选择；
- `--page-size` 1-1000（默认 100）与 `--max-items`（默认 500）；
- `--max-artifact-bytes` 硬上限 256 MiB，以及 `--max-total-bytes`（默认 512 MiB）；
- 应用条目之间的 `--delay-ms`（默认 100）；
- `--resume-cursor <uuid:kind>` 在最后扫描过的组合之后继续。

达到页或条目上限时 `scan_complete: false`，返回最后扫描组合的游标，而不是最后
成功组合。跳过按类别报告，例如 `active_item`、`no_group`、`group_mismatch`、
`inactive_backend`、`invalid_shape`、`oversize`、`missing_declared_sha`、
`invalid_declared_sha`、`sha_mismatch`、`inline_present`、`already_externalized`、
`no_inline`、`changed_ref` 和 `race_changed`。意外的存储、SQL 或完整性错误会用
脱敏汇总停止整批并以非零状态退出，绝不会被当作跳过。

报告只包含计数、字节总量、有界的条目 id 与类型样例；预览明确标注
`input_validation: not_performed`。它绝不打印负载内容、文件名、URL、摘要值或
凭据。

## 安全说明

- 预览只读元数据，不上传、不下载对象，也不写数据库。
- 写入需要已部署产物感知版本。恢复内联数据并不会让旧应用版本变得安全：旧迁移器
  可能拒绝包含更新已应用迁移的账本，而且没有 down migration。考虑旧版本前，需
  静止所有产物写入方并验证部署与 schema 兼容方案。
- 正向写入释放的数据库空间可被 PostgreSQL 复用，但不会自行缩小数据文件。不要在
  本工具流程中执行 `VACUUM FULL`，也不要手工删除 S3 对象或 WAL。
- 读取一个 JSONB 字段仍会让 PostgreSQL 对该行做 detoast。客户端每个条目每种类型
  最多保留一个受限字段和一个受限解码缓冲区。
- 测试中的本地文件系统验证证明工具与往返流程，不能替代生产 S3 验证；该工具绝不
  会自动对生产运行。
