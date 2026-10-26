//! Window state kept between runs.

use gtk::glib;

/// Window state kept between runs, so closing the UI to save memory does
/// not lose the user's place.
#[derive(Debug, Default, PartialEq)]
pub(super) struct SavedState {
    pub(super) width: i32,
    pub(super) height: i32,
    pub(super) maximized: bool,
    pub(super) chat: Option<String>,
}

pub(super) fn state_path() -> std::path::PathBuf {
    glib::user_state_dir().join("wassup").join("ui.ini")
}

impl SavedState {
    /// Missing or unreadable state just means defaults.
    pub(super) fn load(path: &std::path::Path) -> Self {
        let file = glib::KeyFile::new();
        if file.load_from_file(path, glib::KeyFileFlags::NONE).is_err() {
            return Self::default();
        }
        Self {
            width: file.integer("window", "width").unwrap_or(0),
            height: file.integer("window", "height").unwrap_or(0),
            maximized: file.boolean("window", "maximized").unwrap_or(false),
            chat: file
                .string("window", "chat")
                .ok()
                .map(String::from)
                .filter(|chat| !chat.is_empty()),
        }
    }

    pub(super) fn save(&self, path: &std::path::Path) -> Result<(), Box<dyn std::error::Error>> {
        let file = glib::KeyFile::new();
        file.set_integer("window", "width", self.width);
        file.set_integer("window", "height", self.height);
        file.set_boolean("window", "maximized", self.maximized);
        file.set_string("window", "chat", self.chat.as_deref().unwrap_or_default());
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        file.save_to_file(path)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn saved_state_round_trips() {
        let path = std::env::temp_dir()
            .join(format!("wassup-state-{}", std::process::id()))
            .join("ui.ini");
        assert_eq!(SavedState::load(&path), SavedState::default());
        let state = SavedState {
            width: 1000,
            height: 700,
            maximized: true,
            chat: Some("123@g.us".into()),
        };
        state.save(&path).unwrap();
        assert_eq!(SavedState::load(&path), state);
        std::fs::remove_dir_all(path.parent().unwrap()).unwrap();
    }
}
