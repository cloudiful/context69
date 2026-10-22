# 查询元数据与组合门

[English](sqlx-metadata.md)

Context69 的 SQL 针对已提交的查询缓存编译。`sqlx::query_file*!` 从 `.sql` 文件读取
语句并与 `.sqlx` 中的条目匹配，因此受检语句与已提交缓存必须保持一致：语句的字节决定
缓存条目的标识。

## 归属

每个 crate 拥有自身语句的元数据：

- 仓库根目录的工作区缓存（`<workspace root>/.sqlx`）归 Context69 各 crate 所有；
- 固定版本的 `cloudiful-tasks` 依赖在自身清单旁自带 `.sqlx`，其语句元数据归该依赖
  所有。它由 `cargo metadata` 解析；下方组合门要求公共仓库
  `https://github.com/cloudiful/tasks` 的
  `rev = 739cad5822e2208e9a4424eead9f79653a7e02d7`，因此不硬编码任何缓存路径，也不
  允许白名单外的其他 crate。

依赖的元数据绝不复制进工作区缓存，也绝不为消除原生差异而重新生成工作区缓存。被复制
的条目会被报告为“库元数据被吸收进应用”。

## 原生检查

```sh
DATABASE_URL=postgres://… \
  cargo sqlx --no-dotenv prepare --workspace --check -- \
    --all-targets --features integration-test-helpers
```

它需要在环境中显式提供 `DATABASE_URL`：`.env` 加载已由 CLI 自身的 `--no-dotenv`
关闭，且 URL 绝不打印。该命令用真实数据库编译工作区，并把新描述的查询与已提交缓存
比较。

SQLx 0.9 会把工作区外依赖的查询宏也描述进该清单。当依赖在同一次运行中被编译时，清单
也包含它的语句，检查便会以如下信息失败：

```
error: prepare check failed: .sqlx is missing one or more queries; you should re-run sqlx prepare
```

即使工作区缓存与工作区源码完全一致。热构建时依赖不会被重新编译，清单只含工作区语句，
命令以 `0` 退出。对这个固定版本而言两种结果都属正常；原生退出码按实报告，红色的原生
结果绝不会被描述为绿色。

## 组合门

```sh
DATABASE_URL=postgres://… python3 scripts/check-sqlx-metadata.py
python3 scripts/check-sqlx-metadata.py --self-test
```

该门用 `cargo metadata --locked --offline` 解析依赖、要求上述固定版本、在关闭 dotenv
的情况下运行原生命令，并证明三个条件：

- **A1** 每个已提交的工作区条目都由本次原生运行重新生成且逐字节一致，没有缺失、过期
  或未使用的条目，并且每个条目的文件名都是其查询字节的 sha256；
- **A2** 原生运行产生而工作区缓存没有的每个条目，都属于固定库自带的 `.sqlx` 且逐字节
  一致，该库之外没有缺失元数据，工作区缓存中也没有库条目；
- **A3** 带 helper 特性的加锁离线工作区构建在两份缓存原样存在的情况下通过。

比较始终使用原生运行刚刚生成的清单，两种原生结果都是如此。当已提交缓存是该清单的严格
超集时 SQLx 仍以 `0` 退出，因此已提交缓存绝不会与自身比较：清单缺失或不可读时，无论
原生退出码如何，该门都会失败。

它打印 `native_exit`、`native_result`、消费者与依赖条目计数、仅生成条目数与缺失数、
哈希不匹配数、离线构建结果、A1/A2 结论，最后是 `composed_gate=PASS` 或
`composed_gate=FAIL`；仅当全部条件成立才以 `0` 退出。Cargo 自身的 stderr（例如 package
cache 锁等待）作为 `metadata_stderr` 诊断保留，绝不会当作数据解析：只有 `cargo metadata`
的 stdout 会被读作 JSON。元数据失败或 stdout 无法解析时，会给出不带 traceback 的简明
错误。原生失败只被接受为编译成功之后
的那条特定“missing one or more queries”报告；数据库、网络或编译失败会让该门失败。该门
不写入任何缓存：检查模式清单留在构建目录中，且它打印的一切都经过凭据与 URL 脱敏。

`--self-test` 用内存数据、不访问 cargo 或数据库地运行该门自身的夹具用例（未知多余条目、
消费者条目缺失、JSON 不一致、哈希不匹配、依赖版本错误、原生数据库失败、编译失败、过期
检查清单、脱敏）。

## 变更缓存

修改受检 SQL 后，运行 DEV 的 prepare 命令并提交产生的条目：

```sh
DATABASE_URL=postgres://… \
  cargo sqlx --no-dotenv prepare --workspace -- \
    --all-targets --features integration-test-helpers
```

之后 `check-sqlx-metadata.py` 就是“工作区缓存、固定库缓存与离线构建三者一致”的证明。
仅凭原生检查不能构成该证明，因为它的退出码取决于依赖是否被重新编译。
