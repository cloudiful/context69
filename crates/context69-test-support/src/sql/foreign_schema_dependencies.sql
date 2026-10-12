-- Resolve catalog objects to their owning schema before following pg_depend.
-- An unmapped dependent or owned class is returned as a refusal marker, so an
-- unsupported catalog class cannot make DROP SCHEMA ... CASCADE look safe.
WITH owned_schema AS (
    SELECT n.oid AS schema_oid
    FROM pg_catalog.pg_namespace AS n
    WHERE n.nspname = $1
),
objects(classid, objid, namespace_oid) AS (
    SELECT 'pg_catalog.pg_class'::regclass::oid AS classid,
           c.oid AS objid,
           c.relnamespace AS namespace_oid
    FROM pg_catalog.pg_class AS c
    UNION ALL
    SELECT 'pg_catalog.pg_proc'::regclass::oid AS classid,
           p.oid AS objid,
           p.pronamespace AS namespace_oid
    FROM pg_catalog.pg_proc AS p
    UNION ALL
    SELECT 'pg_catalog.pg_type'::regclass::oid AS classid,
           t.oid AS objid,
           t.typnamespace AS namespace_oid
    FROM pg_catalog.pg_type AS t
    UNION ALL
    SELECT 'pg_catalog.pg_constraint'::regclass::oid AS classid,
           constraint_row.oid AS objid,
           constraint_row.connamespace AS namespace_oid
    FROM pg_catalog.pg_constraint AS constraint_row
    UNION ALL
    SELECT 'pg_catalog.pg_rewrite'::regclass::oid AS classid,
           r.oid AS objid,
           rel.relnamespace AS namespace_oid
    FROM pg_catalog.pg_rewrite AS r
    JOIN pg_catalog.pg_class AS rel ON rel.oid = r.ev_class
    UNION ALL
    SELECT 'pg_catalog.pg_trigger'::regclass::oid,
           trigger_row.oid,
           rel.relnamespace
    FROM pg_catalog.pg_trigger AS trigger_row
    JOIN pg_catalog.pg_class AS rel ON rel.oid = trigger_row.tgrelid
    UNION ALL
    SELECT 'pg_catalog.pg_attrdef'::regclass::oid,
           attrdef_row.oid,
           rel.relnamespace
    FROM pg_catalog.pg_attrdef AS attrdef_row
    JOIN pg_catalog.pg_class AS rel ON rel.oid = attrdef_row.adrelid
    UNION ALL
    SELECT 'pg_catalog.pg_policy'::regclass::oid,
           policy_row.oid,
           rel.relnamespace
    FROM pg_catalog.pg_policy AS policy_row
    JOIN pg_catalog.pg_class AS rel ON rel.oid = policy_row.polrelid
    UNION ALL
    SELECT 'pg_catalog.pg_operator'::regclass::oid,
           operator_row.oid,
           operator_row.oprnamespace
    FROM pg_catalog.pg_operator AS operator_row
    UNION ALL
    SELECT 'pg_catalog.pg_opclass'::regclass::oid,
           opclass_row.oid,
           opclass_row.opcnamespace
    FROM pg_catalog.pg_opclass AS opclass_row
    UNION ALL
    SELECT 'pg_catalog.pg_opfamily'::regclass::oid,
           opfamily_row.oid,
           opfamily_row.opfnamespace
    FROM pg_catalog.pg_opfamily AS opfamily_row
    UNION ALL
    SELECT 'pg_catalog.pg_collation'::regclass::oid,
           collation_row.oid,
           collation_row.collnamespace
    FROM pg_catalog.pg_collation AS collation_row
    UNION ALL
    SELECT 'pg_catalog.pg_conversion'::regclass::oid,
           conversion_row.oid,
           conversion_row.connamespace
    FROM pg_catalog.pg_conversion AS conversion_row
    UNION ALL
    SELECT 'pg_catalog.pg_statistic_ext'::regclass::oid,
           statistic_row.oid,
           statistic_row.stxnamespace
    FROM pg_catalog.pg_statistic_ext AS statistic_row
    UNION ALL
    SELECT 'pg_catalog.pg_ts_config'::regclass::oid,
           config_row.oid,
           config_row.cfgnamespace
    FROM pg_catalog.pg_ts_config AS config_row
    UNION ALL
    SELECT 'pg_catalog.pg_ts_dict'::regclass::oid,
           dict_row.oid,
           dict_row.dictnamespace
    FROM pg_catalog.pg_ts_dict AS dict_row
    UNION ALL
    SELECT 'pg_catalog.pg_ts_parser'::regclass::oid,
           parser_row.oid,
           parser_row.prsnamespace
    FROM pg_catalog.pg_ts_parser AS parser_row
    UNION ALL
    SELECT 'pg_catalog.pg_ts_template'::regclass::oid,
           template_row.oid,
           template_row.tmplnamespace
    FROM pg_catalog.pg_ts_template AS template_row
    UNION ALL
    SELECT 'pg_catalog.pg_default_acl'::regclass::oid,
           default_acl.oid,
           default_acl.defaclnamespace
    FROM pg_catalog.pg_default_acl AS default_acl
    UNION ALL
    SELECT 'pg_catalog.pg_extension'::regclass::oid,
           extension_row.oid,
           extension_row.extnamespace
    FROM pg_catalog.pg_extension AS extension_row
    UNION ALL
    SELECT 'pg_catalog.pg_amop'::regclass::oid,
           operator_row.oid,
           opfamily_row.opfnamespace
    FROM pg_catalog.pg_amop AS operator_row
    JOIN pg_catalog.pg_opfamily AS opfamily_row
      ON opfamily_row.oid = operator_row.amopfamily
    UNION ALL
    SELECT 'pg_catalog.pg_amproc'::regclass::oid,
           procedure_row.oid,
           opfamily_row.opfnamespace
    FROM pg_catalog.pg_amproc AS procedure_row
    JOIN pg_catalog.pg_opfamily AS opfamily_row
      ON opfamily_row.oid = procedure_row.amprocfamily
    UNION ALL
    SELECT 'pg_catalog.pg_enum'::regclass::oid,
           enum_row.oid,
           type_row.typnamespace
    FROM pg_catalog.pg_enum AS enum_row
    JOIN pg_catalog.pg_type AS type_row ON type_row.oid = enum_row.enumtypid
    UNION ALL
    SELECT 'pg_catalog.pg_transform'::regclass::oid,
           transform_row.oid,
           type_row.typnamespace
    FROM pg_catalog.pg_transform AS transform_row
    JOIN pg_catalog.pg_type AS type_row ON type_row.oid = transform_row.trftype
    UNION ALL
    SELECT 'pg_catalog.pg_namespace'::regclass::oid,
           own.schema_oid,
           own.schema_oid
    FROM owned_schema AS own
), owned_toast_relations AS (
    SELECT parent.oid AS parent_oid,
           toast.oid AS toast_oid,
           toast.relnamespace AS toast_namespace_oid
    FROM owned_schema AS own
    JOIN pg_catalog.pg_class AS parent
      ON parent.relnamespace = own.schema_oid
    JOIN pg_catalog.pg_class AS toast
      ON toast.oid = parent.reltoastrelid
    JOIN pg_catalog.pg_namespace AS toast_namespace
      ON toast_namespace.oid = toast.relnamespace
    WHERE parent.relkind = 'r'
      AND toast.relkind = 't'
      AND toast_namespace.nspname = 'pg_toast'
), owned_toast_indexes AS (
    SELECT toast.toast_oid,
           index_relation.oid AS index_oid,
           toast.toast_namespace_oid
    FROM owned_toast_relations AS toast
    JOIN pg_catalog.pg_index AS index_row
      ON index_row.indrelid = toast.toast_oid
    JOIN pg_catalog.pg_class AS index_relation
      ON index_relation.oid = index_row.indexrelid
    WHERE index_relation.relkind = 'i'
      AND index_relation.relnamespace = toast.toast_namespace_oid
), allowed_toast_dependencies AS (
    SELECT dependency.classid,
           dependency.objid,
           dependency.refclassid,
           dependency.refobjid
    FROM pg_catalog.pg_depend AS dependency
    JOIN owned_toast_relations AS toast
      ON dependency.classid = 'pg_catalog.pg_class'::regclass::oid
     AND dependency.objid = toast.toast_oid
     AND dependency.refclassid = 'pg_catalog.pg_class'::regclass::oid
     AND dependency.refobjid = toast.parent_oid
     AND dependency.deptype = 'i'
    UNION ALL
    SELECT dependency.classid,
           dependency.objid,
           dependency.refclassid,
           dependency.refobjid
    FROM pg_catalog.pg_depend AS dependency
    JOIN owned_toast_indexes AS index_row
      ON dependency.classid = 'pg_catalog.pg_class'::regclass::oid
     AND dependency.objid = index_row.index_oid
     AND dependency.refclassid = 'pg_catalog.pg_class'::regclass::oid
     AND dependency.refobjid = index_row.toast_oid
     AND dependency.deptype = 'a'
), unclassified_owned AS (
    SELECT '<unclassified owned object>' AS dependency
    FROM pg_catalog.pg_depend AS d
    JOIN owned_schema AS own
      ON d.refclassid = 'pg_catalog.pg_namespace'::regclass
     AND d.refobjid = own.schema_oid
    LEFT JOIN objects AS owned_object
      ON owned_object.classid = d.classid
     AND owned_object.objid = d.objid
    WHERE owned_object.namespace_oid IS NULL
       OR owned_object.namespace_oid <> own.schema_oid
), external_dependencies AS (
    SELECT COALESCE(dep_ns.nspname::text, '<unclassified dependent object>') AS dependency
    FROM pg_catalog.pg_depend AS d
    JOIN objects AS referenced
      ON referenced.classid = d.refclassid
     AND referenced.objid = d.refobjid
    JOIN owned_schema AS own
      ON own.schema_oid = referenced.namespace_oid
      OR EXISTS (
          SELECT 1
          FROM owned_toast_relations AS toast
          WHERE referenced.classid = 'pg_catalog.pg_class'::regclass::oid
            AND referenced.objid = toast.toast_oid
      )
    LEFT JOIN objects AS dependent
      ON dependent.classid = d.classid
     AND dependent.objid = d.objid
    LEFT JOIN pg_catalog.pg_namespace AS dep_ns
      ON dep_ns.oid = dependent.namespace_oid
    WHERE (dependent.namespace_oid IS NULL
       OR dependent.namespace_oid <> own.schema_oid)
      AND NOT EXISTS (
          SELECT 1
          FROM allowed_toast_dependencies AS allowed
          WHERE allowed.classid = d.classid
            AND allowed.objid = d.objid
            AND allowed.refclassid = d.refclassid
            AND allowed.refobjid = d.refobjid
      )
)
SELECT dependency
FROM external_dependencies
UNION
SELECT dependency
FROM unclassified_owned
ORDER BY 1;
