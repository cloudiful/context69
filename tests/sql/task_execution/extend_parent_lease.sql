UPDATE context69.tasks SET lease_until = now() + interval '1 minute' WHERE id = $1
