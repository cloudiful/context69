# 文档结果

文档结果是调用方产生的、仅追加的 JSON 记录，与某个文档及其在产生时对应的确切
源记录哈希绑定。Context69 只负责存储：它不执行模型、不解释业务 schema，也不在
信封之外校验负载。`stock.news` 或 `stock.disclosure` 之类的 stock 生产者，以及
任何其他有界的不透明生产者，负责自己的 schema 与发布语义。

这是 Agent 引擎（stock 生产者）写入的通用存储契约。Context69 负责摄取、索引与
通用结果存储。

## 信封

`CreateDocumentResultRequest` 包含：

- `producer` —— 有界的不透明标识符，`1..=128` 个字符，取自
  `[A-Za-z0-9._:-]`（规范生产者为 `stock.news` 与 `stock.disclosure`）。
- `result_kind` —— 生产者自定义的类型，字母表与长度限制同上。
- `schema_ref` —— 生产者拥有的 schema 引用，`1..=256` 个字符，非空白。
- `source_record_hash` —— 产生该结果所依据的文档 `record_hash`，非空白，最多
  256 个字符。
- `idempotency_key` —— 调用方自选的幂等键，字母表与长度限制同 `producer`。
- `result` —— 不透明负载；必须是 JSON 对象。
- `provenance`（可选）—— 不含凭据、带类型，且仅限 `model_name`、
  `workflow_version`、`run_id`、`producer_config_hash`（各最多 256 个字符）。
  不接受任何其他字段，也不得在此放置凭据。

整个请求序列化后上限为 256 KiB。信封或 provenance 中的未知字段会被拒绝，而非
忽略。

## 追加与幂等

`POST /v1/groups/by-path/{group_path}/documents/{document_id}/results` 追加一条
结果，无论插入还是精确重放都返回 `200`。

- 归属为已认证的 `author_user_id`；请求中没有作者字段。
- 精确重试——同一文档、作者、生产者与幂等键，且负载与 provenance 语义相等——
  返回原始不可变记录，绝不追加第二行。
- 同一幂等键但信封已变更时返回 `409`。
- 新键的 `source_record_hash` 与文档当前哈希不再匹配时返回 `409`。重放检查先
  执行，因此已受理的投递在文档后续更新后仍可安全重试。
- 不同运行使用不同键并追加不同行；两个生产者复用同一键属于不同命名空间。

追加在单个事务中完成，并锁定文档行，从而与并发的文档更新串行化，因此源版本
检查不会与其后的摄取产生竞态。

## 读取

`GET /v1/groups/by-path/{group_path}/documents/{document_id}/results` 按最新优先
列出单个文档的结果，并返回文档当前的 `source_record_hash`，便于调用方绑定随后
的追加。

- `producer` 与 `result_kind` 为可选过滤条件；`after_id` 从上次返回的 id 续读；
  `limit` 默认 `50`，范围 `1..=100`。
- 读取只列出指定文档，且绝不修改 `documents.metadata_json`。

不存在通用的结果 `DELETE` 或 `UPDATE` 端点；记录不可变。

## 分块来源代

每个返回的分块都带有一个必填的 `record_hash` 字段，标明该分块文本所属的来源代。
对于原文分块，它是该分块自身存储的 `document_chunks.record_hash`；对于译文分块，
它是翻译版本的 `source_record_hash`。

不得分析文本不匹配的消费方应将每个分块的 `record_hash` 与外层文档头部的
`record_hash` 逐一比较，并在任一分块为空或不匹配时拒绝该响应。文档头部与其分块
是分别读取的，因此单个响应并非原子快照：并发更新可能使头部处于较新的代而分块仍
处于旧代，该字段让这种情况可被观察，而不是把头部哈希静默盖到陈旧文本上。字段缺
失会导致反序列化失败，因此带来源空缺的分块永远不会被读到。

## 授权

两个操作都限定在群组范围内。路由群组必须是调用方可见的，且文档必须属于该群组；
不可见或非成员的文档一律按 `404` 处理，因此不会泄露私有文档的成员关系。`POST`
还要求 `Maintainer` 角色（否则 `403`），而任何群组内的已授权读者都可以列出。
追加受调用方访问范围约束，因此范围外的文档不可达。

## 存储与删除

结果存放于 `context69.document_results`。文档引用沿用既有的文档所属数据删除
语义（`ON DELETE CASCADE`）。作者引用为 `ON DELETE RESTRICT`：当用户所著的结
果仍存在时，删除该用户会被阻止，因此结果溯源不会被静默丢弃。规划用户删除时需
考虑此限制。
