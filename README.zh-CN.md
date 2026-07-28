# context69

[![Docker 镜像工作流](https://github.com/cloudiful/context69/actions/workflows/publish-docker-ghcr.yml/badge.svg)](https://github.com/cloudiful/context69/actions/workflows/publish-docker-ghcr.yml)
[![Crates 发布工作流](https://github.com/cloudiful/context69/actions/workflows/publish-crates.yml/badge.svg)](https://github.com/cloudiful/context69/actions/workflows/publish-crates.yml)
[![许可证：Apache-2.0](https://img.shields.io/badge/License-Apache--2.0-blue.svg)](LICENSE)
[![最新版本](https://img.shields.io/github/v/release/cloudiful/context69?sort=semver)](https://github.com/cloudiful/context69/releases)

[English](README.md)

Context69 是一个面向检索的知识库服务，用于导入文档、创建向量索引，并通过 HTTP API 和 MCP 提供搜索能力。

## 功能

- 从 PostgreSQL 数据源导入文本、URL 和文件库文档
- 对文档进行规范化、分块和向量化，并将向量存储到 Qdrant
- 通过 Docling 支持 PDF、DOCX 和 XLSX 转换
- 支持多语言翻译和按语言检索
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

docker run -d \
  --name context69 \
  --restart unless-stopped \
  -p 80:80 \
  -p 8097:8097 \
  -v context69-library:/app/data/library \
  -e CONTEXT69_APP_DB__URL='postgres://user:password@db/context69' \
  ghcr.io/cloudiful/context69:<tag>
```

首次启动后访问 `http://localhost`，在设置页面配置 Qdrant、Embedding 服务及其他可选服务，然后重启容器。`80` 端口提供 Web UI 和 HTTP API，`8097` 端口提供 MCP over HTTP。

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
