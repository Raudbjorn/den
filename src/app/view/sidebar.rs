//! The sidebar (drawn from #9 on): for now just the watched Owners.

use super::State;
use crate::app::view_model::Sidebar;

pub fn build(state: &State<'_>) -> Sidebar {
    Sidebar {
        owners: state
            .config
            .as_ref()
            .map(|c| c.owners.clone())
            .unwrap_or_default(),
    }
}
