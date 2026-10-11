import type { components } from "../../src/generated/openapi";

type AuthMeResponse = components["schemas"]["AuthMeResponse"];
type TaskItemResponse = components["schemas"]["TaskItemResponse"];
type TaskItemStatus = components["schemas"]["TaskItemStatus"];
type TaskPageResponse = components["schemas"]["TaskPageResponse"];
type TaskResponse = components["schemas"]["TaskResponse"];
type TaskStatus = components["schemas"]["TaskStatus"];

const BASE_TIME = Date.parse("2026-01-01T00:00:00.000Z");

function fixtureUuid(prefix: string, ordinal: number): string {
  return `${prefix}-0000-4000-8000-${String(ordinal).padStart(12, "0")}`;
}

export function makeQueueTasks(
  count: number,
  statusFor: (index: number) => TaskStatus = () => "running",
): TaskResponse[] {
  return Array.from({ length: count }, (_, index) => {
    const status = statusFor(index);
    const createdAt = new Date(BASE_TIME + index * 1_000).toISOString();
    const progress = {
      cancelled: status === "cancelled" ? 1 : 0,
      failed: status === "failed" ? 1 : 0,
      queued: status === "queued" ? 1 : 0,
      running: status === "running" ? 1 : 0,
      succeeded: status === "succeeded" ? 200 : 0,
      total: 200,
      waiting: status === "waiting" ? 1 : 0,
    };

    return {
      created_at: createdAt,
      file_name: `synthetic-file-${String(index + 1).padStart(3, "0")}.md`,
      group_path: "/queue-perf",
      kind: "file_batch",
      origin: "manual",
      progress,
      status,
      task_id: fixtureUuid("00000000", index + 1),
      updated_at: createdAt,
    } satisfies TaskResponse;
  });
}

export function makeAuthMeResponse(): AuthMeResponse {
  return {
    user: {
      display_name: "Queue performance fixture",
      is_admin: true,
      login_name: "queue-perf-fixture",
      personal_group_path: "/queue-perf",
      user_id: 58,
    },
  };
}

export function makeTaskPage(
  tasks: TaskResponse[],
  page: number,
  pageSize: number,
): TaskPageResponse {
  const start = (page - 1) * pageSize;
  return {
    items: tasks.slice(start, start + pageSize),
    pagination: {
      page,
      page_size: pageSize,
      total: tasks.length,
      total_pages: Math.ceil(tasks.length / pageSize),
    },
  };
}

export function makeTaskItemPage(taskId: string, offset: number): {
  items: TaskItemResponse[];
  next_cursor: string | null;
} {
  const pageSize = 100;
  const items = Array.from({ length: pageSize }, (_, index) => {
    const ordinal = offset + index;
    const suffix = Number(taskId.slice(-12)) + ordinal + 1;
    const itemStatus: TaskItemStatus = "succeeded";
    return {
      attempt_count: 1,
      created_at: new Date(BASE_TIME + ordinal * 1_000).toISOString(),
      file_name: `synthetic-item-${String(ordinal + 1).padStart(3, "0")}.md`,
      item_id: fixtureUuid("10000000", suffix),
      ordinal,
      retryable: false,
      stage: "indexing",
      status: itemStatus,
    } satisfies TaskItemResponse;
  });

  return { items, next_cursor: offset === 0 ? "next-100" : null };
}
