//! Narrow ownership rules for internal TOAST objects reached by schema reset.

#[derive(Clone, Copy)]
struct Relation {
    namespace: Option<&'static str>,
    kind: Option<char>,
    oid: Option<u32>,
    toast_relation_oid: Option<u32>,
}

#[derive(Clone, Copy)]
struct Dependency {
    dependent: Option<Relation>,
    referenced: Option<Relation>,
    parent: Option<Relation>,
    dependency_type: Option<char>,
    index_table_oid: Option<u32>,
}

fn ordinary_parent() -> Relation {
    Relation {
        namespace: Some("context69"),
        kind: Some('r'),
        oid: Some(10),
        toast_relation_oid: Some(20),
    }
}

fn toast_relation() -> Relation {
    Relation {
        namespace: Some("pg_toast"),
        kind: Some('t'),
        oid: Some(20),
        toast_relation_oid: None,
    }
}

fn is_owned_toast_dependency(edge: &Dependency) -> bool {
    let (Some(dependent), Some(referenced), Some(parent)) =
        (edge.dependent, edge.referenced, edge.parent)
    else {
        return false;
    };
    if parent.namespace != Some("context69") || parent.kind != Some('r') || parent.oid.is_none() {
        return false;
    }

    match (
        dependent.namespace,
        dependent.kind,
        referenced.namespace,
        referenced.kind,
        edge.dependency_type,
    ) {
        (Some("pg_toast"), Some('t'), Some("context69"), Some('r'), Some('i')) => {
            dependent.oid == parent.toast_relation_oid && referenced.oid == parent.oid
        }
        (Some("pg_toast"), Some('i'), Some("pg_toast"), Some('t'), Some('a')) => {
            referenced.oid == parent.toast_relation_oid && edge.index_table_oid == referenced.oid
        }
        _ => false,
    }
}

fn toast_relation_edge() -> Dependency {
    Dependency {
        dependent: Some(toast_relation()),
        referenced: Some(ordinary_parent()),
        parent: Some(ordinary_parent()),
        dependency_type: Some('i'),
        index_table_oid: None,
    }
}

fn toast_index_edge() -> Dependency {
    Dependency {
        dependent: Some(Relation {
            namespace: Some("pg_toast"),
            kind: Some('i'),
            oid: Some(30),
            toast_relation_oid: None,
        }),
        referenced: Some(toast_relation()),
        parent: Some(ordinary_parent()),
        dependency_type: Some('a'),
        index_table_oid: Some(20),
    }
}

#[test]
fn correctly_linked_toast_relation_and_its_index_are_owned() {
    assert!(is_owned_toast_dependency(&toast_relation_edge()));
    assert!(is_owned_toast_dependency(&toast_index_edge()));
}

#[test]
fn toast_allowlist_rejects_unverified_ownership_edges() {
    let mut edge = toast_relation_edge();
    edge.referenced = Some(Relation {
        oid: Some(11),
        ..ordinary_parent()
    });
    assert!(!is_owned_toast_dependency(&edge), "mismatched parent oid");

    let mut edge = toast_relation_edge();
    edge.parent = Some(Relation {
        kind: Some('p'),
        ..ordinary_parent()
    });
    assert!(!is_owned_toast_dependency(&edge), "non-ordinary parent");

    let mut edge = toast_relation_edge();
    edge.dependent = Some(Relation {
        kind: Some('r'),
        ..toast_relation()
    });
    assert!(!is_owned_toast_dependency(&edge), "wrong TOAST relkind");

    let mut edge = toast_relation_edge();
    edge.parent = Some(Relation {
        namespace: Some("public"),
        ..ordinary_parent()
    });
    assert!(!is_owned_toast_dependency(&edge), "wrong parent namespace");

    let mut edge = toast_relation_edge();
    edge.dependency_type = Some('n');
    assert!(!is_owned_toast_dependency(&edge), "wrong dependency type");

    let mut edge = toast_relation_edge();
    edge.dependent = Some(Relation {
        namespace: Some("pg_toast_temp_7"),
        ..toast_relation()
    });
    assert!(!is_owned_toast_dependency(&edge), "wrong TOAST namespace");

    let mut edge = toast_relation_edge();
    edge.parent = Some(Relation {
        toast_relation_oid: Some(21),
        ..ordinary_parent()
    });
    assert!(!is_owned_toast_dependency(&edge), "unlinked TOAST relation");

    let mut edge = toast_index_edge();
    edge.index_table_oid = Some(21);
    assert!(
        !is_owned_toast_dependency(&edge),
        "index is not attached to TOAST"
    );

    let mut edge = toast_index_edge();
    edge.dependency_type = Some('i');
    assert!(
        !is_owned_toast_dependency(&edge),
        "wrong index dependency type"
    );

    let mut edge = toast_relation_edge();
    edge.dependent = None;
    assert!(
        !is_owned_toast_dependency(&edge),
        "unclassified catalog class"
    );
}

#[test]
fn catalog_query_uses_the_same_narrow_toast_ownership_checks() {
    let sql = include_str!("../src/sql/foreign_schema_dependencies.sql").to_ascii_lowercase();
    for condition in [
        "parent.relkind = 'r'",
        "toast.relkind = 't'",
        "toast_namespace.nspname = 'pg_toast'",
        "toast.oid = parent.reltoastrelid",
        "dependency.deptype = 'i'",
        "dependency.deptype = 'a'",
        "dependency.refobjid = toast.parent_oid",
        "index_relation.relkind = 'i'",
        "index_row.indrelid = toast.toast_oid",
        "dependency.refobjid = index_row.toast_oid",
        "referenced.objid = toast.toast_oid",
        "allowed_toast_dependencies as",
        "and not exists",
    ] {
        assert!(
            sql.contains(condition),
            "missing catalog guard: {condition}"
        );
    }
}
