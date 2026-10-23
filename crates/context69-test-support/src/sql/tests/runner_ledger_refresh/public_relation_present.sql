-- Whether one relation of the ordinary schema still exists, so a case can prove
-- a preserved object survived the reset cycles. `$1` is the relation name.
SELECT to_regclass('public.' || $1) IS NOT NULL AS "present!"
