//! semantic-retrieval — opt-in capability plugin for Basalt semantic grounding.
//!
//! Installing this plugin enables host-side embedding, grounding, semantic
//! command-palette ranking, and related-file suggestions.

use basalt_plugin_sdk::prelude::*;

basalt_plugin_meta! {
    name:              "semantic-retrieval",
    version:           env!("CARGO_PKG_VERSION"),
    hook_flags:        0u64,
    provides:          "retrieval:semantic",
    requires:          "",
    file_globs:        "",
    activates_on:      "",
    activation_events: "",
}
