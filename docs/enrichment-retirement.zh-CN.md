# 富集退役

[English](enrichment-retirement.md)

内嵌富集曾是已退役模型流水线写入 `documents.metadata_json` 的 `extraction`
对象。Context69 在正常版本发布中完成退役：标准 SQLx 迁移把有用负载导入为通用文档
结果、移除内嵌键、清除过时标签与索引定义，并写入持久化负载清理任务。发布不需要
运维步骤或额外凭据。

## 迁移

`20261017080707_retire_embedded_enrichment.sql` 在常规 `Database::connect`
迁移步骤中、单个事务内执行。没有受影响数据的数据库（例如全新部署）无需引导
管理员即可通过文档守卫，同时仍会收敛过时的任务标签以及
`extraction`/`extraction.*` 元数据索引定义。

当 `metadata_json` 具有确切顶层键 `extraction` 时文档视为受影响。任何写入前，
只要任一受影响文档不满足以下守卫，迁移就回滚整个事务：

- 存在当前哈希的 `document_versions` 快照，其元数据等于移除 `extraction` 后的
  文档元数据，且 title、summary、source URI 一致；
- 该快照正文非空白；
- 分块存在且全部携带当前 `record_hash`；
- `extraction` 是对象，schema 仅限 `stock.news.v1` 与 `stock.disclosure.v1`，
  且各自的值是符合 256 KiB 结果信封上限的对象；
- 组合的库文件或分节元数据不会重新引入该键；
- 没有进行中的摄取任务项或 Docling 远程任务。

对每个受影响文档与 schema，迁移追加一条导入结果：

- producer 为 `context69.enrichment-import`，result kind 为
  `imported-enrichment`，schema ref 为原始 `stock.news.v1` 或
  `stock.disclosure.v1` 键；
- `source_record_hash` 为文档当前哈希；
- 幂等键为 `sqlx-retire:` 加上文档 id、schema ref、源哈希与规范 JSON 负载的
  SHA-256；
- provenance 为 `workflow_version=context69-enrichment-migration-20261017080707`
  与 `run_id=context69-enrichment-retirement-sqlx`，不含模型或凭据字段；
- 作者是现存 `is_admin` 且 `disabled_at` 为空的最小 id 用户。若存在受影响数据却
  没有这样的管理员，迁移回滚，绝不创建账户或冒充原始作者。同一身份下已存在的行
  必须逐字节匹配；信封不同则中止。

随后同一事务只移除 `documents.metadata_json -> 'extraction'`，记录哈希、规范
快照、分块、翻译、向量与任务历史保持不变。无论是否存在受影响文档，它都只清除
tasks 与 task_items 上过时的 `failure_stage = 'extraction'` 标签，并删除
`extraction`/`extraction.*` 元数据索引定义（其值级联删除）。有实际变更时递增
搜索代次使缓存结果失效一次，并为每个受影响文档插入一条 outbox 行。

## 负载收敛

外部点负载无法加入数据库事务，因此迁移写入
`context69.document_metadata_payload_sync`，只保存文档 id 与固定键
`metadata_index.extraction`。后台 worker 在启动时立即排空该 outbox，之后每 5 秒
执行一次：

- 每轮读取进程级向量运行时；在出现就绪的配对前等待且不丢弃任务。因此延迟启动或
  恢复的 Qdrant 无需重启即可接管，重建或设置变更期间也会暂停新的负载清理，直到
  替换配对就绪；
- 每轮最多 16 个任务，使用 `FOR UPDATE SKIP LOCKED` 与 60 秒租约领取，单个外部
  操作上限 20 秒；
- 先以只读方式重新探测集合，且每个原文点必须存在并携带确切的 `document_id` 与
  `record_hash`；原文点缺失时该次尝试失败，绝不重建；
- 只从这些原文点以及所有译文分块 id（包括已被取代的翻译代次）删除该固定嵌套键。
  已不存在的历史译文点计为无操作；
- 删除前与写入完成前都会重新校验代次，因此任务绝不会针对被取代的集合结算；它保持
  待处理并重试；
- 失败时记录净化后的原因，并在 5 秒后重试，按倍数增至 300 秒上限，无限重试，
  绝不丢弃；
- 只有持有当前租约的 worker 才能写入完成，因此并发 worker 不会同时结算同一任务；
- worker 复用恢复控制器的关闭令牌，随应用一起停止；未配置向量运行时时没有控制器，
  也就不会启动该 worker。

## 运维检查

无需手动步骤。检查收敛情况：

```sql
SELECT count(*) FILTER (WHERE completed_at IS NULL) AS pending,
       count(*) FILTER (WHERE completed_at IS NOT NULL) AS completed
FROM context69.document_metadata_payload_sync;

SELECT id, document_id, payload_key, attempts, next_attempt_at, last_error
FROM context69.document_metadata_payload_sync
WHERE completed_at IS NULL
ORDER BY next_attempt_at, id
LIMIT 50;
```

`attempts` 持续增长且有 `last_error` 的待处理行正在重试；它不会回滚已迁移的源
代次。迁移与 worker 都不会读取文档正文、记录负载内容、调用模型、重新嵌入内容、
抓取外部源或修改记录哈希。
