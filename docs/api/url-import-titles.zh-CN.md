# URL 导入任务标题

[English](url-import-titles.md)

URL 导入支持可选的 `title`，它作为处理队列中的任务主题显示。标题仅属于任务元数据：
不会重命名导入的文档、其库文件或它们的规范标题。

## 直接 URL 导入

`POST /v1/groups/by-path/{group_path}/library/files/import-url`

```json
{
  "url": "https://files.example.test/reports/2026-10-15-0099.PDF",
  "title": "季度申报",
  "options": {
    "metadata": {},
    "source_policy": "retain"
  }
}
```

`title`、`folder_id`、`filename` 和 `media_type` 均为可选。`url` 与 `options`
必填，未知字段会被拒绝。

## URL 批量导入

`POST /v1/groups/by-path/{group_path}/batch/url`

```json
{
  "items": [
    {
      "url": "https://files.example.test/reports/2026-10-15-0099.PDF",
      "title": "季度申报",
      "options": {
        "metadata": {},
        "source_policy": "retain"
      }
    }
  ]
}
```

`POST /v1/tasks` 的 `url_batch` 任务接受相同的 `items` 条目，并携带
`group_path`。两个路由都沿用现有任务负载保存提交的条目，因此现有文档标题投影即可
返回该标题，无需新的数据库结构或投影改动。

## 标题处理

- 去除首尾空白并折叠连续空白，内部单个空格保留。
- 缺失、`null`、空字符串或仅含空白的标题按缺失处理，并在序列化的任务负载中省略。
- 非字符串标题会被拒绝，任何未知字段同样会被拒绝。
- 队列行优先显示标题；当文件名与标题不同时，文件名作为次要文本显示。没有标题时沿用
  现有的文件名、分组和来源回退。
- 现有 URL、下载、去重和队列行为不变。规范文档与文件标题不会被覆盖，去重仍复用同一
  规范文档和文件。

调用方必须自行发送 `title`；服务端不会从 PDF 元数据、文件名或页面内容生成标题。已有
任务不会被追溯改名，也不会执行回填。
