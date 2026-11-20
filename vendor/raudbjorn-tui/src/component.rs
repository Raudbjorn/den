use crate::profile::{ColorProfile, TerminalProfile, apply_profile_area};
use crate::theme::{TerminalPalette, mono_cue};
use crepuscularity_core::{eval::eval_expr, parser::ComponentDef};
use crepuscularity_tui::{
    CrepusError, TemplateContext, TemplateValue, parse_component_file, render_nodes,
};
use ratatui::{Frame, layout::Rect};
use std::{
    collections::{HashMap, HashSet},
    sync::Arc,
};

// den patch: the upstream `ComponentId` (the 28 gallery components), the closed
// four-variant `TemplateFile` enum, `load_embedded` and the per-section
// `normalize_*` helpers are gone. Callers register their own template files
// under names of their choosing (an open namespace), and section names carry no
// hidden behaviour.

/// Directory under which every registered template file is visible to
/// `include` directives, as `templates/<name>.crepus`.
pub const TEMPLATE_DIR: &str = "templates";

/// One registered `.crepus` file, addressed by a caller-chosen name.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct TemplateFile(&'static str);

impl TemplateFile {
    pub const fn new(name: &'static str) -> Self {
        Self(name)
    }

    pub const fn name(self) -> &'static str {
        self.0
    }

    /// The virtual path the file is registered under, for `include`.
    pub fn path(self) -> String {
        format!("{TEMPLATE_DIR}/{}.crepus", self.0)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct TemplateRef {
    pub file: TemplateFile,
    pub section: &'static str,
}

/// A template file's source, as handed to [`TemplateStore::from_sources`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TemplateSource {
    pub file: TemplateFile,
    pub content: &'static str,
}

pub struct ComponentFileEntry {
    pub components: HashMap<String, ComponentDef>,
}

pub struct TemplateStore {
    files: HashMap<TemplateFile, ComponentFileEntry>,
    virtual_files: Arc<HashMap<String, String>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TemplateIssue {
    pub file: TemplateFile,
    pub message: String,
}

#[derive(Debug, thiserror::Error)]
pub enum ComponentError {
    #[error("invalid templates: {}", describe_issues(.0))]
    InvalidTemplates(Vec<TemplateIssue>),
    #[error("missing component {section} in {file:?}")]
    MissingComponent {
        file: TemplateFile,
        section: &'static str,
    },
    #[error("render failed for {section} in {file:?}: {source}")]
    Render {
        file: TemplateFile,
        section: &'static str,
        #[source]
        source: CrepusError,
    },
}

fn describe_issues(issues: &[TemplateIssue]) -> String {
    issues
        .iter()
        .map(|issue| format!("{}: {}", issue.file.path(), issue.message))
        .collect::<Vec<_>>()
        .join("; ")
}

fn name_problem(name: &str) -> Option<&'static str> {
    if name.is_empty() {
        return Some("template file name is empty");
    }
    if name.contains(['/', '\\']) || name.contains("..") {
        return Some("template file name must not contain path separators or `..`");
    }
    None
}

impl TemplateStore {
    /// Parses every source up front. All problems (bad names, duplicate names,
    /// parse errors) are collected and returned together.
    pub fn from_sources(
        sources: impl IntoIterator<Item = TemplateSource>,
    ) -> Result<Self, ComponentError> {
        let mut files = HashMap::new();
        let mut virtual_files = HashMap::new();
        let mut seen = HashSet::new();
        let mut issues = Vec::new();

        for TemplateSource { file, content } in sources {
            if let Some(problem) = name_problem(file.name()) {
                issues.push(TemplateIssue {
                    file,
                    message: problem.to_owned(),
                });
                continue;
            }
            if !seen.insert(file) {
                issues.push(TemplateIssue {
                    file,
                    message: "registered more than once".to_owned(),
                });
                continue;
            }
            virtual_files.insert(file.path(), content.to_owned());

            match parse_component_file(content) {
                Ok(comp_file) => {
                    files.insert(
                        file,
                        ComponentFileEntry {
                            components: comp_file.components,
                        },
                    );
                }
                Err(e) => {
                    issues.push(TemplateIssue {
                        file,
                        message: e.to_string(),
                    });
                }
            }
        }

        if !issues.is_empty() {
            return Err(ComponentError::InvalidTemplates(issues));
        }

        Ok(Self {
            files,
            virtual_files: Arc::new(virtual_files),
        })
    }

    pub fn file_count(&self) -> usize {
        self.files.len()
    }

    pub fn component(&self, template: TemplateRef) -> Result<(), ComponentError> {
        self.parsed_component(template).map(|_| ())
    }

    pub fn virtual_files(&self) -> Arc<HashMap<String, String>> {
        self.virtual_files.clone()
    }

    fn parsed_component(&self, template: TemplateRef) -> Result<&ComponentDef, ComponentError> {
        self.files
            .get(&template.file)
            .and_then(|file| file.components.get(template.section))
            .ok_or(ComponentError::MissingComponent {
                file: template.file,
                section: template.section,
            })
    }
}

pub struct ComponentRenderer {
    store: Arc<TemplateStore>,
}

impl ComponentRenderer {
    pub fn new(store: Arc<TemplateStore>) -> Self {
        Self { store }
    }

    /// Renders a registered section into `area`.
    ///
    /// Order matters: sentinel colours are replaced with `palette` roles first,
    /// then the colour/glyph profile is applied. Pass a palette already meant
    /// for the profile when nearest-colour quantisation would merge roles.
    ///
    /// den patch: a template can no longer be overridden at render time through
    /// `ctx.virtual_files` (upstream re-parsed it on every render and silently
    /// fell back to the store on a parse error), neither as the rendered section
    /// nor as the target of an `include`. Extra virtual files in `ctx` at other
    /// paths remain visible to `include`. A caller-provided `base_dir` is ignored.
    pub fn render(
        &self,
        template: TemplateRef,
        ctx: &TemplateContext,
        palette: TerminalPalette,
        profile: TerminalProfile,
        frame: &mut Frame,
        area: Rect,
    ) -> Result<(), ComponentError> {
        let component = self.store.parsed_component(template)?;
        if area.is_empty() {
            return Ok(());
        }

        let mut render_ctx = ctx.clone();
        // Registered files go in last, so they win over a caller file at the same
        // path: an `include` of a registered template gets the registered source.
        let mut virtual_files = (*render_ctx.virtual_files).clone();
        virtual_files.extend(
            self.store
                .virtual_files()
                .iter()
                .map(|(path, content)| (path.clone(), content.clone())),
        );
        render_ctx.virtual_files = Arc::new(virtual_files);
        render_ctx.base_dir = Some(TEMPLATE_DIR.into());

        render_ctx.set("w", area.width as i64);
        render_ctx.set("h", area.height as i64);
        render_ctx.set("viewport_width", area.width as i64);
        render_ctx.set("viewport_height", area.height as i64);
        for (key, expr) in &component.meta.defaults {
            render_ctx.vars.entry(key.clone()).or_insert_with(|| {
                eval_expr(expr, &TemplateContext::default()).unwrap_or(TemplateValue::Null)
            });
        }
        render_nodes(&component.nodes, &render_ctx, frame, area).map_err(|e| {
            ComponentError::Render {
                file: template.file,
                section: template.section,
                source: e,
            }
        })?;

        let colourless = matches!(profile.color, ColorProfile::Mono | ColorProfile::NoColor);
        let buf = frame.buffer_mut();
        for y in area.top()..area.bottom() {
            for x in area.left()..area.right() {
                if let Some(cell) = buf.cell_mut((x, y)) {
                    if colourless {
                        let cue = mono_cue(cell.fg) | mono_cue(cell.bg);
                        cell.modifier.insert(cue);
                    }
                    cell.fg = palette.get_semantic_color(cell.fg);
                    cell.bg = palette.get_semantic_color(cell.bg);
                    cell.underline_color = palette.get_semantic_color(cell.underline_color);
                }
            }
        }

        apply_profile_area(buf, area, profile);
        Ok(())
    }
}
