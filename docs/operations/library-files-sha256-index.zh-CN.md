# 库文件去重查询索引

[English](library-files-sha256-index.md)

库去重查询按 `(group_id, sha256)` 读取 `context69.library_files`，并以
`ORDER BY created_at, id LIMIT 1` 取确定性的首行。此前没有索引覆盖该列对，
因此每次未命中都会全表扫描。迁移
`20261022140130_library_files_group_sha256_index.sql` 新增一个非唯一 btree
索引 `(group_id, sha256, created_at, id)`：等值列对位于索引前部，后部两列
恰好承载查询所需的顺序，同一个索引同时服务于过滤与排序。

该迁移不改动任何查询文本、响应或唯一性规则，也不触碰任何数据。它只是把既有
查询从全表扫描变为索引查找。

## 迁移做了什么

该迁移是一个普通的事务性迁移文件，在常规 `Database::connect` 迁移步骤中执行：

1. 目录守卫查找同名对象 `context69.idx_library_files_group_sha256`；
2. `CREATE INDEX IF NOT EXISTS idx_library_files_group_sha256 ON
   context69.library_files (group_id, sha256, created_at, id)` 创建索引。

守卫是失败即关闭的，因为当同名关系已存在时，`CREATE INDEX IF NOT EXISTS`
只发出 NOTICE 而不报错。没有守卫时，错误或无效的同名对象会让该语句静默空转，
升级会报告一个索引从未被创建的成功。

仅当同名对象是有效、就绪、在用、非唯一的 btree 索引，且恰好覆盖
`(group_id, sha256, created_at, id)`、采用默认升序、无谓词、无表达式列、
无 INCLUDE 列时，守卫才接受它。其他任何情况都会中止迁移，并给出指明对象与
不匹配属性的错误信息。

迁移绝不会删除、重命名或修复同名对象。如何处置由运维决定。

## 升级前预先创建索引

迁移内的语句是普通 `CREATE INDEX`，因此它会在整个建索引期间持有
`context69.library_files` 上的写锁。在大型库上，这会在建索引结束前阻塞文件摄取
与文件写入。这与早先 `hot_path_indexes` 迁移面临的是同一权衡，办法也相同：在
升级之前，以带外方式并发创建该索引。

由数据库属主在升级前执行：

```sql
CREATE INDEX CONCURRENTLY idx_library_files_group_sha256
    ON context69.library_files (group_id, sha256, created_at, id);
```

`CREATE INDEX CONCURRENTLY` 不能在迁移的事务内运行，因此必须在独立会话中执行。
索引名不加 schema 限定：PostgreSQL 总是把索引建在其表所属的 schema 中，因此
它会落在 `context69` 中，与 `context69.library_files` 相邻。

迁移随后运行时，守卫会找到这个等价索引并接受它，`IF NOT EXISTS` 使该语句成为
空操作。

`CREATE INDEX CONCURRENTLY` 在建索引期间不持有写锁，但它会等待所有正在运行的
事务。并发建索引失败会留下一个无效索引，处置方式见恢复章节。

## 校验索引

升级前与升级后各执行一次：

```sql
SELECT c.relname,
       am.amname AS access_method,
       i.indisunique,
       i.indisvalid,
       i.indisready,
       i.indislive,
       i.indnkeyatts,
       i.indnatts,
       pg_get_indexdef(c.oid) AS definition
FROM pg_catalog.pg_index i
JOIN pg_catalog.pg_class c ON c.oid = i.indexrelid
JOIN pg_catalog.pg_namespace n ON n.oid = c.relnamespace
JOIN pg_catalog.pg_am am ON am.oid = c.relam
WHERE n.nspname = 'context69'
  AND c.relname = 'idx_library_files_group_sha256';
```

期望结果是一行：`access_method` 为 `btree`，`indisunique` 为 false，
`indisvalid`、`indisready`、`indislive` 均为 true，`indnkeyatts` 为 4，
`indnatts` 为 4，定义为

```sql
CREATE INDEX idx_library_files_group_sha256 ON context69.library_files USING btree (group_id, sha256, created_at, id)
```

`indnatts` 大于 `indnkeyatts` 表示索引带有 INCLUDE 列，守卫会拒绝它。
`indisvalid` 为 false 表示索引无效（例如并发建索引失败后残留），守卫同样拒绝。

## 守卫拒绝升级时

迁移以如下形式中止：

```text
context69 library file dedup index guard failed for context69.idx_library_files_group_sha256: <不匹配的属性>. Inspect it with SELECT pg_get_indexdef('context69.idx_library_files_group_sha256'::regclass) and the pg_index/pg_class catalogs. ...
```

整个迁移回滚，迁移账本不会前移，运行中的版本仍停留在上一个版本。数据库中没有被
改动的内容，因此对象处置完成后可直接重试升级。

分两步处置：

1. 用上面的校验查询以及 `pg_get_indexdef` 检查该对象。错误信息已列出不匹配的属性。
2. 由运维决定走哪条路径：
   - 该对象是守卫未能识别的等价索引——仅当校验查询显示定义与有效性符合预期时
     才保留它，否则自行修正定义；
   - 该对象是错误的、无效的，或根本不是索引——由运维自行删除，并用上面的
     `CREATE INDEX CONCURRENTLY` 语句重新创建；或者不留任何同名对象，让迁移
     自行创建。

迁移自身绝不删除或修复同名对象，也绝不猜测原本期望的形状。删除一个可能已被其他
组件使用的同名对象，是运维决策，而不是迁移行为。

## 已应用的迁移不可修改

迁移一旦应用，其文件不得再编辑：迁移账本保存了文件校验和，改动文件会导致下次
升级失败。已发布迁移的修复应放在新的迁移中，而本索引不需要任何修复。该索引
只由一个迁移文件创建，第二个文件会就同一个名字与之冲突。
