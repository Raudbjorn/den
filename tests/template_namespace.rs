//! den's templates live in their own namespace in the engine's open template
//! store, not in the upstream `Views` slot through a virtual-file override.

// Test code: helpers outside #[test] fns may fail loudly too (clippy.toml covers only #[test]).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod support;

use crepuscularity_tui::TemplateContext;
use den::ui::templates;
use raudbjorn_tui::{
    component::{ComponentError, TemplateFile, TemplateRef, TemplateSource, TemplateStore},
    profile::ColorProfile,
};
use std::{collections::HashMap, sync::Arc};

#[test]
fn den_templates_register_only_their_own_files() {
    let store = TemplateStore::from_sources(templates::sources()).expect("den templates load");
    let mut paths: Vec<String> = store.virtual_files().keys().cloned().collect();
    paths.sort();
    assert_eq!(
        paths,
        [
            "templates/detail.crepus",
            "templates/frame.crepus",
            "templates/list.crepus"
        ]
    );
    for section in templates::ALL_SECTIONS {
        assert!(store.component(section).is_ok(), "{section:?} missing");
    }
}

#[test]
fn namespaces_keep_same_named_sections_apart() {
    let one = TemplateFile::new("one");
    let two = TemplateFile::new("two");
    let sources = || {
        vec![
            TemplateSource {
                file: one,
                content: "--- Greeting\ndiv \"from one\"\n",
            },
            TemplateSource {
                file: two,
                content: "--- Greeting\ndiv \"from two\"\n",
            },
        ]
    };
    let profile = support::profile(ColorProfile::TrueColor);
    let ctx = TemplateContext::default();
    for (file, expected) in [(one, "from one"), (two, "from two")] {
        let section = TemplateRef {
            file,
            section: "Greeting",
        };
        let buffer = support::render(sources(), section, &ctx, profile, 12, 1);
        assert_eq!(support::row_text(&buffer, 0).trim_end(), expected);
    }
}

#[test]
fn render_context_cannot_override_a_registered_template() {
    let file = TemplateFile::new("fixed");
    let section = TemplateRef {
        file,
        section: "Body",
    };
    let ctx = TemplateContext {
        virtual_files: Arc::new(HashMap::from([(
            file.path(),
            "--- Body\ndiv \"overridden\"\n".to_owned(),
        )])),
        ..TemplateContext::default()
    };
    let buffer = support::render(
        vec![TemplateSource {
            file,
            content: "--- Body\ndiv \"registered\"\n",
        }],
        section,
        &ctx,
        support::profile(ColorProfile::TrueColor),
        12,
        1,
    );
    assert_eq!(support::row_text(&buffer, 0).trim_end(), "registered");
}

#[test]
fn render_context_cannot_override_a_registered_include() {
    let outer = TemplateFile::new("outer");
    let inner = TemplateFile::new("inner");
    let section = TemplateRef {
        file: outer,
        section: "Body",
    };
    // Same key as the registered `inner` file, so only precedence decides.
    let ctx = TemplateContext {
        virtual_files: Arc::new(HashMap::from([(
            inner.path(),
            "--- Part\ndiv \"overridden\"\n".to_owned(),
        )])),
        ..TemplateContext::default()
    };
    let buffer = support::render(
        vec![
            TemplateSource {
                file: outer,
                content: "--- Body\ndiv\n include inner.crepus#Part\n",
            },
            TemplateSource {
                file: inner,
                content: "--- Part\ndiv \"registered\"\n",
            },
        ],
        section,
        &ctx,
        support::profile(ColorProfile::TrueColor),
        12,
        1,
    );
    assert_eq!(support::row_text(&buffer, 0).trim_end(), "registered");
}

#[test]
fn render_context_files_stay_visible_to_include() {
    let outer = TemplateFile::new("outer");
    let section = TemplateRef {
        file: outer,
        section: "Body",
    };
    let ctx = TemplateContext {
        virtual_files: Arc::new(HashMap::from([(
            TemplateFile::new("extra").path(),
            "--- Part\ndiv \"from ctx\"\n".to_owned(),
        )])),
        ..TemplateContext::default()
    };
    let buffer = support::render(
        vec![TemplateSource {
            file: outer,
            content: "--- Body\ndiv\n include extra.crepus#Part\n",
        }],
        section,
        &ctx,
        support::profile(ColorProfile::TrueColor),
        12,
        1,
    );
    assert_eq!(support::row_text(&buffer, 0).trim_end(), "from ctx");
}

#[test]
fn every_template_problem_is_reported_with_its_file() {
    let good = TemplateFile::new("good");
    let result = TemplateStore::from_sources([
        TemplateSource {
            file: good,
            content: "--- A\ndiv \"a\"\n",
        },
        TemplateSource {
            file: good,
            content: "--- B\ndiv \"b\"\n",
        },
        TemplateSource {
            file: TemplateFile::new("../escape"),
            content: "--- C\ndiv \"c\"\n",
        },
        TemplateSource {
            file: TemplateFile::new(""),
            content: "--- D\ndiv \"d\"\n",
        },
    ]);
    let Err(ComponentError::InvalidTemplates(issues)) = result else {
        panic!("expected InvalidTemplates");
    };
    let files: Vec<&str> = issues.iter().map(|i| i.file.name()).collect();
    assert_eq!(files, ["good", "../escape", ""]);
    assert!(issues[0].message.contains("more than once"));
}

#[test]
fn a_missing_section_is_an_error_not_a_blank() {
    let store = TemplateStore::from_sources(templates::sources()).expect("den templates load");
    let missing = TemplateRef {
        file: templates::FRAME,
        section: "NoSuchSection",
    };
    assert!(matches!(
        store.component(missing),
        Err(ComponentError::MissingComponent { .. })
    ));
}
