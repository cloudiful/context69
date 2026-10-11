use super::super::{SearchCase, SeededFixture, public_scope, request};
use super::case;

pub fn search_cases(fixture: &SeededFixture) -> Vec<SearchCase> {
    let mut cases = Vec::new();
    let mut translated = request(format!("same{}needle", fixture.tag), &fixture.group_path);
    translated.locale = Some("fr".to_owned());
    cases.push(case(
        "same UUID in original and translated chunk tables stays distinct",
        translated,
        public_scope(),
        100,
        2,
        Some("chunk_phrase"),
    ));

    cases.push(case(
        "null locale performs no translation work",
        request(format!("same{}needle", fixture.tag), &fixture.group_path),
        public_scope(),
        100,
        1,
        Some("chunk_phrase"),
    ));

    let mut fallback = request(
        format!("fallback{}needle", fixture.tag),
        &fixture.group_path,
    );
    fallback.locale = Some("fr".to_owned());
    cases.push(case(
        "original fallback carries latest translation job status",
        fallback,
        public_scope(),
        100,
        1,
        Some("chunk_phrase"),
    ));

    let mut stale = request(format!("stale{}needle", fixture.tag), &fixture.group_path);
    stale.locale = Some("fr".to_owned());
    cases.push(case(
        "stale source hash translation is excluded",
        stale,
        public_scope(),
        100,
        0,
        None,
    ));

    for (name, query, count, reason) in [
        (
            "translated title phrase expands eligible chunks",
            "translated title".to_owned(),
            1,
            "title_exact",
        ),
        (
            "translated terms split across title and chunk",
            format!("translated same{}needle", fixture.tag),
            1,
            "all_terms",
        ),
        (
            "translated three terms split across fields",
            format!("translated same{}needle traduit", fixture.tag),
            1,
            "all_terms",
        ),
        (
            "translated duplicate terms preserve intersection",
            format!("translated translated same{}needle", fixture.tag),
            1,
            "all_terms",
        ),
        (
            "translated intersection rejects missing term",
            format!(
                "translated same{}needle absent{}term",
                fixture.tag, fixture.tag
            ),
            0,
            "all_terms",
        ),
    ] {
        let mut translated = request(query, &fixture.group_path);
        translated.locale = Some("fr".to_owned());
        cases.push(case(
            name,
            translated,
            public_scope(),
            100,
            count,
            Some(reason),
        ));
    }
    let mut other_locale = request(
        format!("translated same{}needle", fixture.tag),
        &fixture.group_path,
    );
    other_locale.locale = Some("de".to_owned());
    cases.push(case(
        "requested locale excludes other translation candidates",
        other_locale,
        public_scope(),
        100,
        0,
        None,
    ));

    for (name, query) in [
        (
            "private translated phrase candidates obey authorization",
            format!("private{}title", fixture.tag),
        ),
        (
            "private translated term candidates obey authorization",
            format!("private{}title private{}chunk", fixture.tag, fixture.tag),
        ),
    ] {
        let mut translated = request(query, &fixture.private_group_path);
        translated.locale = Some("fr".to_owned());
        cases.push(case(name, translated.clone(), public_scope(), 100, 0, None));
        cases.push(case(
            name,
            translated,
            context69::domain::AccessScope {
                private_group_ids: vec![fixture.private_group_id],
                ..public_scope()
            },
            100,
            1,
            None,
        ));
    }

    let mut boundary = request(
        format!("{} boundaryneedle", fixture.tag),
        &fixture.group_path,
    );
    boundary.source_key = Some("fixture-source".to_owned());
    cases.push(case(
        "global ranked limit retains the full candidate boundary",
        boundary,
        public_scope(),
        80,
        80,
        Some("chunk_phrase"),
    ));
    cases
}
