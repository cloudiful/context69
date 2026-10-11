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
- 任务历史默认保留，由用户清理（逐条回收站/恢复/彻底删除，批量清空已完成/回收站）；管理员取消进行中任务与 Docling 恢复保留在服务端
- 提供 HTTP API、MCP（HTTP 或 stdio）以及可选的 Web UI

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

## 许可证

Apache-2.0，详见 [LICENSE](LICENSE)。
