# context69

[![Docker 镜像工作流](https://github.com/cloudiful/context69/actions/workflows/publish-docker-ghcr.yml/badge.svg)](https://github.com/cloudiful/context69/actions/workflows/publish-docker-ghcr.yml)
[![Release 工作流](https://github.com/cloudiful/context69/actions/workflows/release.yml/badge.svg)](https://github.com/cloudiful/context69/actions/workflows/release.yml)
[![许可证：Apache-2.0](https://img.shields.io/badge/License-Apache--2.0-blue.svg)](LICENSE)
[![最新版本](https://img.shields.io/github/v/release/cloudiful/context69?sort=semver)](https://github.com/cloudiful/context69/releases)

[English](README.md)

Context69 是一个面向检索的知识库服务，用于导入文档、创建向量索引，并通过 HTTP API 和 MCP 提供搜索能力。

## 功能

- 从 PostgreSQL 数据源导入文本、URL 和文件库文档
- 对文档进行规范化、分块和向量化，并将向量存储到 Qdrant
- 通过 Docling 支持 PDF、DOCX 和 XLSX 转换
- 支持多语言翻译和按语言检索
- 任务历史默认保留，由用户清理（逐条回收站/恢复/彻底删除）；处理队列支持跨页勾选任务并按当前筛选条件捕获整个匹配集合，执行带确认的批量重试/继续/取消/移入回收站，回收站内可批量恢复或彻底删除；管理员取消进行中任务与 Docling 恢复保留在服务端
- 提供 HTTP API、MCP（HTTP 或 stdio）以及可选的 Web UI

### 运行时切块

切块预算可以按字符或按令牌计量。令牌模式使用明确选择的分词器（`cl100k_base`、
`o200k_base`、固定预设，或已验证的本地 Hugging Face `tokenizer.json` 产物），重叠
部分始终保持在单个切块的预算内。切块区域只有一个分词器控件，同时服务于两种预算
单位；嵌入输入范围在保存时由它推导，因此切块预算与输入检查不会不一致。保存后的
预算立即作用于新建任务，无需重启；已经开始的任务保留其创建时的预算，已存储的文档
不会被自动重新切块。

固定预设是一个固定的上游 `tokenizer.json`：仓库、修订、大小、SHA256 和许可证都是
常量。准备所选预设只会把该文件下载到可写缓存目录，在可用之前校验精确大小与哈希，
并在运行中的进程内立即生效，无需重启。许可证与来源链接会在开始准备之前显示。产品
既不内置也不再分发该文件，上游许可证也不代表允许商业使用。缓存目录为
`./data/tokenizers.cache`，可用 `CONTEXT69_CHUNKING__TOKENIZER_DOWNLOAD_DIR` 覆盖；
已准备的预设在重启后会从该已验证缓存离线加载。就绪状态属于单个进程，因此每个副本
都需要该已验证缓存文件才能使用该预设。

挂载产物仍是运维自备文件的高级路径：将标准 `tokenizer.json` 放在
`<artifact-dir>/<小写sha256>.json`，并以只读方式挂载该目录；默认目录为
`./data/tokenizers`，可用 `CONTEXT69_CHUNKING__TOKENIZER_ARTIFACT_DIR` 覆盖。进程在
启动时校验文件名哈希和可读分词器，设置页只提供已验证的产物，并在所选哈希缺失时
失败关闭。新增或移除挂载产物需要重启；已有任务固定过的哈希在其仍可重试期间必须保留。

### 嵌入输入范围

嵌入输入策略是独立的统计约定：切块所选的分词器与哈希、可选的完整输入令牌上限、
可选的文档与查询服务端前缀，以及 Hugging Face 特殊令牌开关。HTTP API 仍保留独立的
分词器与哈希字段，非 UI 客户端仍可选择不同的统计分词器。前缀表示服务商在分词前
自行附加的文本；它们与输入一起精确编码统计，绝不发送、裁剪或拆分。
`max_item_chars` 仍是独立的高级请求守卫，不会关闭令牌范围。

完整统计输入超过上限的文档或查询会在任何 HTTP 请求前被拒绝，绝不截断或静默重新
切块。需要时按 UTF-8 边界缩短文档切块，未输出的源字节保留在流中。任务会随切块
预算固定该策略，因此之后的设置保存不会改变运行中的任务；没有策略的快照保持原有
的不设防行为。

## 部署

### Docker 镜像

发布的镜像位于 GHCR。将 `<tag>` 替换为版本标签，例如 `v0.8.1`。

依赖：

- PostgreSQL：存储应用数据
- Qdrant 和 Embedding 服务：提供搜索和文档导入能力
- Valkey：可选，用于共享会话和调度协调
- Docling：可选，处理 PDF、DOCX 或 XLSX 时需要

```bash
docker volume create context69-library

# 只生成一次主密钥，安全保存，并在所有副本上传递同一个值。
export CONTEXT69_MASTER_SECRET="$(openssl rand -base64 32)"

docker run -d \
  --name context69 \
  --restart unless-stopped \
  -p 80:80 \
  -p 8097:8097 \
  -v context69-library:/app/data/library \
  -e DATABASE_URL='postgres://user:password@db/context69' \
  -e CONTEXT69_APP__MASTER_SECRET="$CONTEXT69_MASTER_SECRET" \
  ghcr.io/cloudiful/context69:<tag>
```

首次启动后访问 `http://localhost`，在设置页面配置 Qdrant、Embedding 服务及其他可选服务，然后重启容器。`80` 端口提供 Web UI 和 HTTP API，`8097` 端口提供 MCP over HTTP。

运行时设置区为未保存的 Qdrant 地址提供测试连接按钮。它使用与索引相同的 gRPC 链路检查可达性（通常是 `6334` 端口，而非 REST `6333` 端口），5 秒超时，只提示成功或脱敏后的失败，不会保存设置、通知观察者，也不要求集合已存在。

Embedding 区将现有模型称为文档模型，并可填写可选的查询模型；留空时查询沿用文档模型。同一区域还包含四项手动请求上限（单请求输入数、单输入字符数、单请求字符数、单请求字节数）。

修改 Embedding 模型、网关或维度即改变向量空间。现有集合会继续提供服务，直到显式执行“重建向量索引”并激活目标集合；在配置身份与活跃身份不一致期间，搜索和入库不可用，操作者仍可取消重建并保留当前集合。全新安装没有可保留的集合，因此首次索引构建始终是显式操作。重建状态、暂停、继续与取消都是持久化的管理员操作：暂停会让已提交批次先完成再停靠，继续从最后写入的分页之后接着执行，取消则终止本次尝试且不改动现有集合。处理队列可以单独暂停接收新任务；运行中的任务继续执行。

### 存储的密钥

`CONTEXT69_APP__MASTER_SECRET` 是 Base64 编码的 32 字节密钥，用于加密所有已存储的密钥——Embedding/Search/Docling/翻译服务 API Key、源连接 URL、Git 凭据、Webhook 签名密钥以及浏览器会话签名密钥。只生成一次，请像保管其他凭据一样保管它：没有它，已加密的密钥将无法读取回。轮换它需要执行已配置的密钥轮换流程，而不是重启。

在已有密钥的部署上设置它会转换它找到的内容。启动时，在任何凭据被读取之前，Context69 会使用已配置的密钥分批加密所有仍以旧版明文形式存储的密钥。转换按行、按批次原子进行：如果中途停止，已完成的批次保持加密，下次启动会从剩余部分继续。如果某个已存储的密钥无法加密（例如其密钥名不被当前版本识别），启动会失败，因此部署不会在密钥实际未受保护时却自认为已受保护。未配置主密钥的部署不受影响：其密钥仍可像以前一样以明文读取。

有两点它不会做。这是启动时的转换，不是持续保护，因此**每个会写入密钥的副本都必须配置同一个主密钥**（或者在一个副本转换期间停止其余副本）；没有该密钥运行的实例可能在转换完成后写入新的明文密钥。转换之前的备份仍然包含明文密钥——**context69 不会清理已有备份**，请自行按需处理并按计划淘汰它们。

### 从源码构建

```bash
cargo build --release --bin context69
cd frontend
bun install --no-save
bun run build
cd ..
mkdir -p ci-image-input/frontend-dist
install -Dm755 target/release/context69 ci-image-input/context69
cp -R frontend/dist/. ci-image-input/frontend-dist/
docker build -t context69:latest .
```

运行本地镜像时使用上面的 Docker 命令，并将镜像名替换为 `context69:latest`。

## 后端测试

纯测试可在任意环境运行：

```bash
cargo test --workspace
```

依赖数据库的测试被显式标记为 `#[ignore]`，因此上面的命令只运行纯测试，并把被忽略的测试如实报告出来，而不会伪装成通过。下方介绍在已配置测试数据库上运行它们的工具与显式重置流程。

运行器的常规调用会先以 Cargo 默认并行度执行 `cargo test --workspace --no-fail-fast --offline`，其中也包括 doctest。被忽略的数据库测试和可选测试不会被选中。纯测试失败时，运行器会在解析数据库目标或开始任何数据库预检、加锁、重置之前停止。`--preflight` 与 `--list` 仍是检查模式，不会执行纯测试套件。

运行器从 `CONTEXT69_TEST_DATABASE_URL` 读取目标；若该变量未设置，则读取仓库 `.env` 文件中的 `DATABASE_URL`。它不会把数据库 URL 作为参数接收，不会创建或删除数据库，也不会自行部署 PostgreSQL 服务。仅仅存在环境变量 `DATABASE_URL` 不会选中任何目标。

`./scripts/test-backend.sh --preflight` 只读检查目标库的 schema、迁移账本与扩展，并报告运行器是否允许重置它。`./scripts/test-backend.sh --list` 在完全不连接数据库的前提下，将仓库中登记的数据库测试清单与编译产物进行核对。

重置目标库需要单独、明确的授权。包装脚本会原样转发参数，但不会替你加上它；只有你显式要求时，运行器才会删除并重建 `context69` schema：

```bash
./scripts/test-backend.sh --preflight         # 只读，报告重置范围
./scripts/test-backend.sh --authorize-reset    # 纯测试、重置、迁移，然后执行数据库测试
```

默认情况下，其他客户端连接都会阻止重置。在操作者确认的单用户测试窗口中，可在预检和执行时都传入 `--allow-idle-connections`，仅允许统计信息可见、无事务的 idle 会话。活动会话、事务中会话、信息被屏蔽或未知的会话以及预备事务仍会阻止重置。空闲会话可能在检查后变为活动状态，因此不能在应用写入进程存在时使用该选项。该选项不能与 `--list` 组合，也不代表同意重置。

```bash
./scripts/test-backend.sh --preflight --allow-idle-connections
./scripts/test-backend.sh --authorize-reset --allow-idle-connections
```

运行产物与各测试目标的日志写入 `target/backend-test-results/<run-id>/`。

## 许可证

Apache-2.0，详见 [LICENSE](LICENSE)。
