SELECT table_name, column_name, is_nullable, column_default
           FROM information_schema.columns
          WHERE table_schema = 'context69'
            AND table_name IN ('task_items', 'task_attempts')
            AND column_name IN ('execution_task_id', 'execution_generation', 'execution_attempt_id')
          ORDER BY table_name, column_name
