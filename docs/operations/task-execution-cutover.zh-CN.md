# 任务执行切换

[English](task-execution-cutover.md)

任务中心通过 `cloudiful_tasks` 库执行其条目。每个条目都携带一段已捕获的执行分段，
因此恰好有一个执行权威拥有它：不再持有该分段的 worker 无法写入，被杀掉的 worker
的租约会过期，而不是留下半完成的行。在库拥有执行之前创建的条目完全没有分段。

缺失的分段正是本次切换要解决的问题。两个权威绝不能运行同一个条目，因此只要存在
没有分段的条目，任务引擎就拒绝启动，由运维显式完成交接。

## 所有权划分

库拥有条目的执行租约与尝试生命周期：其分段决定哪个 worker 可以写入，未被续期的
租约会过期。域内行只是镜像该租约。父任务的准入、进而条目的可执行性，仍是域内决定：
关联分段不等于准入，因此父任务无法获得槽位的条目会保持排队。条目负载（包括其
`indexing_checkpoint`）在已捕获的授权下写入，因此被取代的代次无法覆盖其后继者的
检查点。

未执行自身异步 drop 就停止的进程绝不会释放它持有的租约，也没有其他东西代该进程
释放：常规过期路径会中断库尝试，并把失败投影到条目上。交接保留它找到的每一个产物、
尝试行和负载。它只关联分段或关闭被中断的条目，绝不删除历史、对象引用或条目的
取证轨迹。

## 启动闩锁

每次 `TaskService` 启动只做一次就绪判定，在首次调度与任何维护写入之前：

- `cloudiful_tasks` schema 必须存在、归库所有且兼容；并且
- 不得有任何 `queued`、`waiting` 或 `running` 条目缺少执行分段。

判定被阻塞时，任务引擎什么都不启动：没有认领调度器、没有认领维护、没有 Docling
恢复、接管或清理，也没有任务源清理调度器。HTTP、手动任务 CRUD 和独立服务保持可用。
就绪结果连同阻塞代码、未关联数量与修复提示，在默认 `context69=info` 过滤下只记录
一次；绝不重新评估，因此在运行中的进程下执行的交接不会在其之下打开引擎。交接完成后
重启进程。

`/healthz` 与 `/readyz` 通过既有的 `library_processing_ready` 字段报告同一进程闩锁，
被阻塞时返回 `503`。`/livez` 不受影响：即使引擎关闭，进程仍然存活。

可写的常规启动会在应用迁移之后通过库自带的 migrate API 准备库 schema，`db_init`
也显式执行同一准备。可写常规启动遇到缺失的 schema 会在此创建；升级恢复准入、只读
连接与运维连接绝不迁移或创建它。schema 检查失败关闭：外来或不兼容的库 schema 会让
启动显式失败，而不是被接管，也不会在阻塞引擎的情况下打开第二个权威。

## 交接工具

`cargo run --bin task_execution_cutover -- <command> [flags]`

该工具只从 `DATABASE_URL` 读取连接串；没有 `.env` 回退，也不接受 URL 参数。它连接时
不执行迁移，也不做升级准入，因此不执行任何 DDL。URL 中配置的数据库名与服务器报告的
名称都必须等于必填的 `--expect-database`，并且在进行任何读写之前校验库 schema。

`--workers-stopped` 对两条交接命令都是必填的。它是运维断言，并且工具还会在两个权威
仍持有存活租约时拒绝写入：存在带存活镜像租约的运行中条目，或存在对仍然存在的条目
持有存活租约的库尝试。交接前停止所有任务 worker。

每条命令打印一行 JSON 元数据，成功时以 `0` 退出，用法错误或数据库目标不匹配时以 `2`
退出，数据库或安全拒绝时以 `3` 退出。所有消息（包括驱动错误）都会剥离凭据。

### `inventory`

只读，也是默认的第一步。它列出阻塞引擎的条目，最多一页（至多 100 行），并给出决定
每行需要何种处理所需的执行元数据：条目与父任务 id、状态、父任务类型、序号、父任务
是否已退役、执行代次与链接、重试截止时间，以及条目是否在等待 Docling 依赖。不选择
条目负载与用户数据。

```
task_execution_cutover inventory --expect-database context69 --limit 100 --offset 0
```

### `adopt`

为显式指定、尚未关联的 `queued` 或 `waiting` 条目关联一段全新的执行分段。
`--item <uuid>` 可重复，每次调用至多 100 个唯一 id。`--all` 从阻塞集合中选择一批
有界且可接管的条目，而不是显式点名；它与 `--item` 互斥，且绝不选择 `running`、
终端、已关联或父任务已退役的条目。契约如下：

- 每批一个事务，在既有准入锁和按确定顺序获取的条目锁下执行。
- 条目的域内状态保持不变：`waiting` 条目保持等待，因此仅有新分段绝不会绕过其父任务
  的准入，Docling 等待仍需先完成对账才能变为可执行。重试截止时间、尝试次数、依赖
  键、阶段、资源与 Docling 远程引用均不变。
- 每个被指定的父任务都必须是活动的且未退役。`--all` 会跳过已不再合格的候选，而不是
  让整批失败；显式 `--item` 仍会在条目不合格时让整批失败。
- 对已携带分段的条目重复执行接管是空操作：不会产生第二段分段，也不会有新代次。
- `running`、终端、父任务已退役或其他不合格的显式条目会让整批原子地失败；无法取得
  的关联会回滚该分段，而不是留下没有任何条目指向的分段。
- `--all` 在应用批次上限之前于 SQL 中选择合格行，因此任意数量的更早不合格行都不会
  挤占可接管工作。它报告 `remaining_adoptable`（父任务活动的未关联 `queued`/
  `waiting` 条目）与 `total_blockers`（全部未关联非终态条目）。仅当
  `remaining_adoptable` 非零时重复执行。`total_blockers` 还包含需要显式
  `finalize-interrupted --item` 的 `running` 条目，以及两个命令都无法解决的父任务
  已退役条目；应报告这些阻塞项，而不要声称 `adopt --all` 能清除它们。命令自身不会
  循环到耗尽。

```
task_execution_cutover adopt --expect-database context69 --workers-stopped \
  --item 0f2c... --item 5b71...
```

```
task_execution_cutover adopt --expect-database context69 --workers-stopped --all
```

### `finalize-interrupted`

关闭显式指定、尚未关联且域内租约已经消失的 `running` 条目。这些条目属于在切换前
停止的 worker，工具将它们移出阻塞集合而不重放：

- 条目以租约阶段和错误 `interrupted at task execution cutover` 失败，其未关闭的域内
  尝试变为带完成时间戳的 `interrupted`。
- 父任务计数与状态在同一事务中重算，文件状态沿用既有的失败投影。
- 不重新排队、不重放，也不创建新分段。之后的 retry 是显式用户操作，会按常规策略
  打开自己的分段；retry 标记也遵循该策略：仍有尝试预算的条目保持可重试，已耗尽的
  则不可。
- 已关联、非运行中或仍持有租约的条目会让整批失败。

```
task_execution_cutover finalize-interrupted --expect-database context69 \
  --workers-stopped --item 0f2c...
```

## 操作步骤

1. 停止所有任务 worker。
2. 运行 `inventory`，逐条决定采用 `adopt` 还是 `finalize-interrupted`。
3. 用确切的条目 id 运行所选命令，每批至多 100 个；或用 `adopt --all` 交接一批有界
   的可接管条目。
4. 重复 `inventory`（或在 `adopt --all` 的 `remaining_adoptable` 非零时重复它），直到
   不再有可接管行；对 `adopt` 无法解决的 `total_blockers`，通过
   `finalize-interrupted` 处理或明确报告。
5. 重新启动进程；启动时准备库 schema，启动闩锁此时报告就绪，任务引擎接管。
