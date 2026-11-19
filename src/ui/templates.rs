//! den's own `.crepus` templates, registered in the engine's open template
//! namespace (no gallery components, no ride in another file's slot).
//!
//! To add a template file: put it in `templates/`, add a `TemplateFile` constant
//! and a `TemplateSource` entry to [`sources`]. Section names are the `--- Name`
//! headings inside the file.

use raudbjorn_tui::component::{TemplateFile, TemplateRef, TemplateSource};

/// The Tonal-plates frame: header and footer plates, sutures, chamfer.
pub const FRAME: TemplateFile = TemplateFile::new("frame");
/// The Next Step list and its empty state.
pub const LIST: TemplateFile = TemplateFile::new("list");
/// The detail plate.
pub const DETAIL_FILE: TemplateFile = TemplateFile::new("detail");
/// The sidebar plate (Repos and Scopes).
pub const SIDEBAR_FILE: TemplateFile = TemplateFile::new("sidebar");

/// Every template file den registers.
pub fn sources() -> [TemplateSource; 4] {
    [
        TemplateSource {
            file: FRAME,
            content: include_str!("../../templates/frame.crepus"),
        },
        TemplateSource {
            file: LIST,
            content: include_str!("../../templates/list.crepus"),
        },
        TemplateSource {
            file: DETAIL_FILE,
            content: include_str!("../../templates/detail.crepus"),
        },
        TemplateSource {
            file: SIDEBAR_FILE,
            content: include_str!("../../templates/sidebar.crepus"),
        },
    ]
}

const fn section(file: TemplateFile, section: &'static str) -> TemplateRef {
    TemplateRef { file, section }
}

pub const HEADER: TemplateRef = section(FRAME, "Header");
pub const FOOTER: TemplateRef = section(FRAME, "Footer");
pub const SUTURE_V: TemplateRef = section(FRAME, "SutureV");
pub const CHAMFER: TemplateRef = section(FRAME, "Chamfer");
pub const LIST_ROWS: TemplateRef = section(LIST, "List");
pub const LIST_EMPTY: TemplateRef = section(LIST, "Empty");
pub const DETAIL: TemplateRef = section(DETAIL_FILE, "Detail");
pub const SIDEBAR: TemplateRef = section(SIDEBAR_FILE, "Sidebar");

/// Every section den draws.
pub const ALL_SECTIONS: [TemplateRef; 8] = [
    HEADER, FOOTER, SUTURE_V, CHAMFER, LIST_ROWS, LIST_EMPTY, DETAIL, SIDEBAR,
];
