UPDATE context69.task_attempts SET status = 'interrupted', finished_at = now() WHERE id = $1
