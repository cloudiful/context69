SELECT COUNT(*)::BIGINT AS "count!"
FROM context69.personal_access_tokens
WHERE user_id = $1
