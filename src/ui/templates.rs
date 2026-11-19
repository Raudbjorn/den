//! den's own `.crepus` templates, registered in the engine's open template
//! namespace (no gallery components, no ride in another file's slot).
//!
//! To add a template file: put it in `templates/`, add a `TemplateFile` constant
//! and a `TemplateSource` entry to [`sources`]. Section names are the `--- Name`
//! headings inside the file.

use raudbjorn_tui::component::{TemplateFile, TemplateRef, TemplateSource};

/// The Tonal-plates frame: header, list, detail and footer plates, sutures, chamfer.
pub const FRAME: TemplateFile = TemplateFile::new("frame");

/// Every template file den registers.
pub fn sources() -> [TemplateSource; 1] {
    [TemplateSource {
        file: FRAME,
        content: include_str!("../../templates/frame.crepus"),
    }]
}

const fn frame(section: &'static str) -> TemplateRef {
    TemplateRef {
        file: FRAME,
        section,
    }
}

pub const HEADER: TemplateRef = frame("Header");
pub const LIST_EMPTY: TemplateRef = frame("ListEmpty");
pub const DETAIL: TemplateRef = frame("Detail");
pub const FOOTER: TemplateRef = frame("Footer");
pub const SUTURE_V: TemplateRef = frame("SutureV");
pub const CHAMFER: TemplateRef = frame("Chamfer");
