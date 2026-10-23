# Docling 远端任务与清理

Docling 设置提供两个独立的只读视图和一个显式清理唤醒操作。两个列表均仅限管理员。

## 远端任务

`GET /v1/admin/docling/tasks` 读取已保存的 Docling 服务。它只读地展示共享服务，
因此可能包含其他客户端提交的任务；它不代表 Context69 的归属，也不代表完整枚举。
分页使用上游 `limit`/`offset` 契约与 `has_more`；不提供客户端总数、筛选或排序。
界面提供 20、50、100 三档每页数量，并且只显示上游的 `created_at` 时间。

## 本地清理记录

`GET /v1/admin/docling/cleanup` 对持久化的本地清理队列分页：

- `page` 从 1 开始，`page_size` 默认 20，最大 100。
- `status`、`outcome`、`remote_task_id` 为可选精确筛选。任务 ID 会去除首尾空白，
  且不允许为空；不支持通配符或子串匹配。
- 响应包含当前页和真实的服务端 `total`。排序为 `created_at` 降序，再按 `id` 降序。
- 每行提供持久化的 `created_at`、`updated_at` 和可选的 `finished_at`，以及原有的
  尝试次数、下次尝试时间和期限。

持久化状态值为 `pending`、`cancelling`、`awaiting_terminal`、`done`、
`unsupported`、`failed`。持久化结果值为 `success`、`partial_success`、`failure`、
`skipped`、`cancelled`、`not_found`。缺失的结果保持为空，不会推断新值。

界面保持筛选与页码独立：修改筛选或每页数量会回到第 1 页，刷新保留当前选择，
较旧查询的迟到响应不会覆盖当前页。

## 清理唤醒

`POST /v1/admin/docling/cleanup` 会唤醒所有符合条件的本系统已停止任务，而不是
仅当前筛选页或可见页。它不会停止正在执行的工作，也不会中断已开始的转换。仅
期限内的待处理、取消中或等待终态记录会受影响；已完成、不支持、失败及过期记录
保持终态。`not_found` 表示观察到远端任务不可见，并不代表已确认取消。

旧版 `GET /v1/admin/docling/tasks` 响应仍保留其已弃用的 `cleanup` 数组和原有
线上格式，以便旧调用方继续工作。
